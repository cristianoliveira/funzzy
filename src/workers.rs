use crate::executor::{Run, Step};
use crate::rules::{self, Rules};
use crate::stdout;
use crate::template;
use std::path::PathBuf;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::mpsc::{channel, Receiver, Sender};
use std::thread::JoinHandle;
use std::time::Duration;

#[derive(Debug, Clone)]
pub enum WorkerEvent {
    Started {
        run_id: u64,
        trigger: String,
        commands: Vec<String>,
    },
    Finished {
        elapsed: Duration,
        failures: Vec<String>,
    },
    Cancelled,
    Tick,
}

/// A run requested through the worker's command stream.
struct RunRequest {
    run_id: u64,
    commands: Vec<rules::CommandLine>,
    trigger: String,
}

/// Scheduling and replacement flow through one ordered stream, so a newer run
/// always supersedes any queued work instead of racing a separate cancel
/// channel.
enum WorkerCommand {
    Run(RunRequest),
    Cancel,
}

/// Drain the command stream keeping only the newest message, so intermediate
/// queued generations are discarded before any process spawn.
fn drain_newest(receiver: &Receiver<WorkerCommand>) -> Option<WorkerCommand> {
    let mut newest = None;
    while let Ok(command) = receiver.try_recv() {
        newest = Some(command);
    }
    newest
}

pub struct Worker {
    scheduler: Option<Sender<WorkerCommand>>,
    next_run_id: AtomicU64,
    root: PathBuf,

    consumer: Option<JoinHandle<()>>,
}

impl Worker {
    /// Convenience constructor resolving the workspace root from the process
    /// current directory. Keep usage at the outer boundary; prefer
    /// [`Worker::with_root`] so command template preparation does not depend
    /// on hidden process state.
    pub fn new<F>(verbose: bool, fail_fast: bool, on_event: F) -> Self
    where
        F: Fn(WorkerEvent) + Send + 'static,
    {
        let root = std::env::current_dir().expect("Unable to get current directory");
        Self::with_root(verbose, fail_fast, root, on_event)
    }

    /// Creates a worker that expands command templates against an explicit
    /// workspace root.
    pub fn with_root<F>(verbose: bool, fail_fast: bool, root: PathBuf, on_event: F) -> Self
    where
        F: Fn(WorkerEvent) + Send + 'static,
    {
        stdout::verbose("Worker in verbose mode.", verbose);
        let (tscheduler, rscheduler) = channel::<WorkerCommand>();

        let consumer = std::thread::spawn(move || {
            let mut active: Option<Run> = None;
            let mut pending: Option<RunRequest> = None;

            loop {
                if active.is_none() {
                    // Promote the newest superseding run, or block on the next
                    // command when idle.
                    if let Some(req) = pending.take() {
                        on_event(WorkerEvent::Started {
                            run_id: req.run_id,
                            trigger: req.trigger.clone(),
                            commands: req
                                .commands
                                .iter()
                                .map(|command| command.display())
                                .collect(),
                        });
                        active = Some(Run::new(req.commands, fail_fast));
                        continue;
                    }

                    match rscheduler.recv() {
                        Ok(WorkerCommand::Run(req)) => {
                            on_event(WorkerEvent::Started {
                                run_id: req.run_id,
                                trigger: req.trigger.clone(),
                                commands: req
                                    .commands
                                    .iter()
                                    .map(|command| command.display())
                                    .collect(),
                            });
                            active = Some(Run::new(req.commands, fail_fast));
                        }
                        Ok(WorkerCommand::Cancel) => {}
                        Err(_) => break,
                    }
                    continue;
                }

                let step = active.as_mut().expect("active run").advance();
                match step {
                    Step::Running => match drain_newest(&rscheduler) {
                        Some(WorkerCommand::Run(req)) => {
                            let mut replaced = active.take().expect("active run");
                            replaced.cancel(verbose);
                            on_event(WorkerEvent::Cancelled);
                            pending = Some(req);
                        }
                        Some(WorkerCommand::Cancel) => {
                            let mut cancelled = active.take().expect("active run");
                            cancelled.cancel(verbose);
                            on_event(WorkerEvent::Cancelled);
                        }
                        None => {
                            let current_task = active
                                .as_ref()
                                .and_then(|run| run.current_task().map(str::to_string))
                                .unwrap_or_default();
                            stdout::verbose(
                                &format!("waiting next tick for task: {}", current_task),
                                verbose,
                            );
                            on_event(WorkerEvent::Tick);
                            std::thread::sleep(Duration::from_millis(200));
                        }
                    },
                    Step::Finished => {
                        let finished = active.take().expect("active run");
                        let elapsed = finished.elapsed();
                        let results = finished.into_results();
                        let failures = results
                            .iter()
                            .filter_map(|result| result.as_ref().err().cloned())
                            .collect();
                        stdout::present_results(results, elapsed);
                        on_event(WorkerEvent::Finished { elapsed, failures });
                    }
                }
            }

            stdout::info("Consumer thread finished.");
        });

        Worker {
            scheduler: Some(tscheduler),
            next_run_id: AtomicU64::new(0),
            root,
            consumer: Some(consumer),
        }
    }

    pub fn cancel_running_tasks(&self) -> Result<(), String> {
        if let Some(scheduler) = self.scheduler.as_ref() {
            if let Err(err) = scheduler.send(WorkerCommand::Cancel) {
                println!("failed to send cancel signal: {:?}", err);
                return Err(format!("{:?}", err));
            }
        }

        Ok(())
    }

    pub fn schedule(&self, rules: Vec<Rules>, filepath: &str) -> Result<u64, String> {
        self.schedule_with_trigger(rules, filepath, Some(filepath))
    }

    pub(crate) fn schedule_with_trigger(
        &self,
        rules: Vec<Rules>,
        trigger: &str,
        filepath: Option<&str>,
    ) -> Result<u64, String> {
        if let Some(scheduler) = self.scheduler.as_ref() {
            let expanded: Vec<rules::CommandLine> = rules::command_lines(rules)
                .into_iter()
                .map(|command| {
                    let expanded = template::template_line(
                        command,
                        template::TemplateOptions {
                            filepath: filepath.map(str::to_string),
                            current_dir: format!("{}", self.root.display()),
                        },
                    );
                    for variable in &expanded.unknown_variables {
                        stdout::warn(&format!("Unknown template variable '{}'.", variable));
                    }
                    expanded.command
                })
                .collect();
            let run_id = self.next_run_id.fetch_add(1, Ordering::Relaxed) + 1;
            let request = RunRequest {
                run_id,
                commands: expanded,
                trigger: trigger.to_string(),
            };
            if let Err(err) = scheduler.send(WorkerCommand::Run(request)) {
                return Err(format!("{:?}", err));
            }
            return Ok(run_id);
        }

        Err("worker scheduler is unavailable".to_string())
    }
}

impl Drop for Worker {
    fn drop(&mut self) {
        if let Some(scheduler) = self.scheduler.as_ref() {
            let _ = scheduler.send(WorkerCommand::Cancel);
        }

        let ts = self.scheduler.take();
        drop(ts);
        if let Some(th) = self.consumer.take() {
            th.join().expect("failed to join consumer thread");
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::path::PathBuf;
    use std::sync::mpsc::{channel, Receiver};
    use std::time::Instant;

    fn output_file(name: &str) -> PathBuf {
        std::env::temp_dir().join(format!("funzzy-worker-{}-{}", std::process::id(), name))
    }

    fn write_file_rule(path: &PathBuf) -> Rules {
        Rules::new(
            "test".to_string(),
            vec![format!("echo triggered > {}", path.display())],
            vec!["src/**/*.rs".to_string()],
            vec![],
            false,
        )
    }

    fn worker_with_events(verbose: bool, fail_fast: bool) -> (Worker, Receiver<WorkerEvent>) {
        let (tx, rx) = channel();
        (
            Worker::new(verbose, fail_fast, move |event| {
                tx.send(event).unwrap();
            }),
            rx,
        )
    }

    fn rule(commands: Vec<&str>) -> Rules {
        Rules::new(
            "test".to_string(),
            commands.into_iter().map(str::to_string).collect(),
            vec![],
            vec![],
            false,
        )
    }

    fn expect_event<F>(rx: &Receiver<WorkerEvent>, what: &str, pred: F) -> WorkerEvent
    where
        F: Fn(&WorkerEvent) -> bool,
    {
        let deadline = Instant::now() + Duration::from_secs(10);
        while Instant::now() < deadline {
            match rx.recv_timeout(Duration::from_millis(100)) {
                Ok(event) if pred(&event) => return event,
                Ok(_) => {}
                Err(_) => {}
            }
        }
        panic!("timed out waiting for {}", what);
    }

    fn collect_until_finished(rx: &Receiver<WorkerEvent>) -> Vec<WorkerEvent> {
        let mut events = vec![];
        loop {
            let event = rx
                .recv_timeout(Duration::from_secs(10))
                .expect("timed out waiting for worker to finish a run");
            let finished = matches!(event, WorkerEvent::Finished { .. });
            events.push(event);
            if finished {
                return events;
            }
        }
    }

    #[test]
    fn burst_replacement_runs_only_the_newest_generation() {
        let (worker, rx) = worker_with_events(false, false);

        let slow = rule(vec!["sleep 5"]);
        let quick = rule(vec!["echo ok"]);

        let first = worker.schedule(vec![slow.clone()], "a.txt").unwrap();
        expect_event(
            &rx,
            "first run to start",
            |e| matches!(e, WorkerEvent::Started { run_id, .. } if *run_id == first),
        );
        // The consumer is now polling the active child; wait for a tick so both
        // follow-up schedules are queued before the next replacement drain.
        expect_event(&rx, "worker tick", |e| matches!(e, WorkerEvent::Tick));

        let second = worker.schedule(vec![slow], "b.txt").unwrap();
        let third = worker.schedule(vec![quick], "c.txt").unwrap();

        let events = collect_until_finished(&rx);
        drop(worker);

        let started: Vec<u64> = events
            .iter()
            .filter_map(|e| match e {
                WorkerEvent::Started { run_id, .. } => Some(*run_id),
                _ => None,
            })
            .collect();

        assert_eq!(
            started,
            vec![third],
            "only the newest generation may start after the replacement"
        );
        assert!(
            !started.contains(&second),
            "intermediate generations must be discarded before process spawn"
        );
        assert!(
            events.iter().any(|e| matches!(e, WorkerEvent::Cancelled)),
            "the superseded run must be cancelled"
        );
        assert!(
            events
                .iter()
                .any(|e| matches!(e, WorkerEvent::Finished { .. })),
            "the newest generation must finish"
        );
    }

    #[test]
    fn replaced_run_never_executes_remaining_commands() {
        let output = output_file("replaced-remaining");
        let _ = std::fs::remove_file(&output);

        let (worker, rx) = worker_with_events(false, false);
        let slow = Rules::new(
            "test".to_string(),
            vec![
                "sleep 5".to_string(),
                format!("echo must-not-run > {}", output.display()),
            ],
            vec![],
            vec![],
            false,
        );
        let first = worker.schedule(vec![slow], "a.txt").unwrap();
        expect_event(
            &rx,
            "first run to start",
            |e| matches!(e, WorkerEvent::Started { run_id, .. } if *run_id == first),
        );

        worker
            .schedule(vec![rule(vec!["echo ok"])], "b.txt")
            .unwrap();
        collect_until_finished(&rx);
        drop(worker);

        assert!(
            !output.exists(),
            "superseded run must not execute commands after cancellation"
        );
    }

    #[test]
    fn explicit_cancel_terminates_the_active_run() {
        let (worker, rx) = worker_with_events(false, false);
        let run_id = worker
            .schedule(vec![rule(vec!["sleep 5"])], "a.txt")
            .unwrap();

        expect_event(
            &rx,
            "run to start",
            |e| matches!(e, WorkerEvent::Started { run_id: id, .. } if *id == run_id),
        );

        worker.cancel_running_tasks().unwrap();

        expect_event(&rx, "run to be cancelled", |e| {
            matches!(e, WorkerEvent::Cancelled)
        });
        drop(worker);
        assert!(
            matches!(rx.recv_timeout(Duration::from_millis(300)), Err(_)),
            "a cancelled run must never emit Finished"
        );
    }

    #[test]
    fn fail_fast_stops_after_first_failed_command() {
        let output = output_file("fail-fast");
        let _ = std::fs::remove_file(&output);

        let (worker, rx) = worker_with_events(true, true);
        let commands = vec![
            "false".to_string(),
            format!("echo must-not-run > {}", output.display()),
        ];
        worker
            .schedule(
                vec![Rules::new(
                    "test".to_string(),
                    commands,
                    vec![],
                    vec![],
                    false,
                )],
                "a.txt",
            )
            .unwrap();

        collect_until_finished(&rx);
        drop(worker);

        assert!(
            !output.exists(),
            "fail-fast must skip remaining commands after a failure"
        );
    }

    #[test]
    fn without_fail_fast_later_commands_still_run_after_a_failure() {
        let output = output_file("no-fail-fast");
        let _ = std::fs::remove_file(&output);

        let (worker, rx) = worker_with_events(false, false);
        let commands = vec![
            "false".to_string(),
            format!("echo ran > {}", output.display()),
        ];
        worker
            .schedule(
                vec![Rules::new(
                    "test".to_string(),
                    commands,
                    vec![],
                    vec![],
                    false,
                )],
                "a.txt",
            )
            .unwrap();

        let events = collect_until_finished(&rx);
        drop(worker);

        assert!(
            output.exists(),
            "later commands must run when fail-fast is disabled"
        );
        let failures: Vec<String> = match events.last().unwrap() {
            WorkerEvent::Finished { failures, .. } => failures.clone(),
            _ => vec![],
        };
        assert_eq!(failures.len(), 1, "the single failure must be reported");
    }

    #[test]
    fn it_templates_relative_filepath_against_the_injected_root() {
        let marker = output_file("injected-root");
        let _ = std::fs::remove_file(&marker);

        let root =
            std::env::temp_dir().join(format!("funzzy root with spaces {}", std::process::id()));
        let (tx, rx) = channel();
        let worker = Worker::with_root(false, false, root.clone(), move |event| {
            tx.send(event).unwrap();
        });

        let rule = Rules::new(
            "test".to_string(),
            vec![format!(
                "echo '{{{{relative_filepath}}}}' > {}",
                marker.display()
            )],
            vec![],
            vec![],
            false,
        );
        let filepath = root.join("src/main.rs");
        worker
            .schedule_with_trigger(
                vec![rule],
                filepath.to_str().unwrap(),
                Some(filepath.to_str().unwrap()),
            )
            .unwrap();

        collect_until_finished(&rx);
        drop(worker);

        let content = std::fs::read_to_string(&marker).expect("marker file was not written");
        assert_eq!(
            content.trim(),
            "src/main.rs",
            "template expansion must be relative to the injected root"
        );
        let _ = std::fs::remove_file(&marker);
    }

    #[test]
    fn it_does_not_run_scheduled_tasks_when_worker_is_dropped() {
        let output = output_file("dropped");
        let _ = std::fs::remove_file(&output);

        {
            let worker = Worker::new(false, false, |_| {});
            let rule = Rules::new(
                "test".to_string(),
                vec![
                    "sleep 1".to_string(),
                    format!("echo triggered > {}", output.display()),
                ],
                vec!["src/**/*.rs".to_string()],
                vec![],
                false,
            );
            worker.schedule(vec![rule], "src/main.rs").unwrap();
        }

        std::thread::sleep(std::time::Duration::from_millis(1500));
        assert!(!output.exists(), "dropped worker should not run hooks");
    }

    #[test]
    fn it_runs_scheduled_tasks_without_cancel_signal() {
        let output = output_file("scheduled");
        let _ = std::fs::remove_file(&output);

        {
            let worker = Worker::new(false, false, |_| {});
            worker
                .schedule(vec![write_file_rule(&output)], "src/main.rs")
                .unwrap();
            std::thread::sleep(std::time::Duration::from_millis(300));
        }

        assert!(output.exists(), "scheduled hook should run");
        let _ = std::fs::remove_file(&output);
    }
}
