extern crate nix;

use crate::cmd::spawn;
use crate::cmd::LoggedChild;
use crate::rules::{self, Rules};
use crate::stdout;
use nix::sys::signal::{self, Signal};
use nix::unistd::Pid;
use std::collections::VecDeque;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::mpsc::{channel, Receiver, Sender};
use std::thread::JoinHandle;
use std::time::{Duration, Instant};

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
    commands: Vec<String>,
    trigger: String,
}

/// Scheduling and replacement flow through one ordered stream, so a newer run
/// always supersedes any queued work instead of racing a separate cancel
/// channel.
enum WorkerCommand {
    Run(RunRequest),
    Cancel,
}

/// A run being executed. `advance` spawns commands in order and polls the
/// active child; `cancel` gracefully terminates the current child and discards
/// the remaining commands.
struct ActiveRun {
    commands: VecDeque<String>,
    results: Vec<Result<(), String>>,
    started: Instant,
    child: Option<LoggedChild>,
    current_task: Option<String>,
    fail_fast: bool,
}

enum Step {
    /// A child is executing; the consumer may poll for replacements.
    Running,
    /// Every command finished (or fail-fast stopped the run).
    Finished,
}

impl ActiveRun {
    fn new(req: RunRequest, fail_fast: bool) -> Self {
        ActiveRun {
            commands: req.commands.into(),
            results: vec![],
            started: Instant::now(),
            child: None,
            current_task: None,
            fail_fast,
        }
    }

    /// Advance this run by one step: spawn the next command or poll the active
    /// child. Returns `Running` whenever a child is executing.
    fn advance(&mut self) -> Step {
        loop {
            if self.child.is_none() {
                let Some(task) = self.commands.pop_front() else {
                    return Step::Finished;
                };
                self.current_task = Some(task.clone());
                match spawn(&task) {
                    Ok(child) => {
                        self.child = Some(child);
                        return Step::Running;
                    }
                    Err(err) => {
                        let failure = format!("Command {} failed to start: {}", task, err);
                        stdout::error(&failure);
                        self.results.push(Err(failure));
                        if self.fail_fast {
                            return Step::Finished;
                        }
                        continue;
                    }
                }
            }

            let task = self.current_task.clone().unwrap_or_default();
            match self.child.as_mut().expect("child is running").try_wait() {
                Ok(None) => return Step::Running,
                Ok(Some(status)) => {
                    self.child = None;
                    self.current_task = None;
                    if status.success() {
                        self.results.push(Ok(()));
                    } else {
                        self.results
                            .push(Err(format!("Command {} has failed with {}", task, status)));
                        if self.fail_fast {
                            return Step::Finished;
                        }
                    }
                }
                Err(err) => {
                    self.child = None;
                    self.current_task = None;
                    self.results
                        .push(Err(format!("Command {} has errored with {}", task, err)));
                    if self.fail_fast {
                        return Step::Finished;
                    }
                }
            }
        }
    }

    /// Gracefully terminate the active child, if any. The run never finishes
    /// normally after this; remaining commands are discarded.
    fn cancel(&mut self, verbose: bool) {
        if let Some(child) = self.child.as_mut() {
            let task = self.current_task.clone().unwrap_or_default();
            stdout::verbose(&format!("---- cancelling: {:?} ----", task), verbose);

            if let Err(err) = signal::kill(
                Pid::from_raw(child.id() as i32),
                // Sends a SIGTERM signal to the process
                // and allows it to exit gracefully.
                Signal::SIGTERM,
            ) {
                stdout::error(&format!("failed to terminate task {:?}: {:?}", task, err));
            }

            if let Ok(status) = child.wait() {
                stdout::verbose(
                    &format!("---- finished: {:?} status: {} ----", task, status),
                    verbose,
                );
            } else {
                stdout::error(&format!(
                    "failed to wait for the task to finish: {:?}",
                    task
                ));
            }
        }
        self.child = None;
        self.commands.clear();
    }
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

    consumer: Option<JoinHandle<()>>,
}

impl Worker {
    pub fn new<F>(verbose: bool, fail_fast: bool, on_event: F) -> Self
    where
        F: Fn(WorkerEvent) + Send + 'static,
    {
        stdout::verbose("Worker in verbose mode.", verbose);
        let (tscheduler, rscheduler) = channel::<WorkerCommand>();

        let consumer = std::thread::spawn(move || {
            let mut active: Option<ActiveRun> = None;
            let mut pending: Option<RunRequest> = None;

            loop {
                if active.is_none() {
                    // Promote the newest superseding run, or block on the next
                    // command when idle.
                    if let Some(req) = pending.take() {
                        on_event(WorkerEvent::Started {
                            run_id: req.run_id,
                            trigger: req.trigger.clone(),
                            commands: req.commands.clone(),
                        });
                        active = Some(ActiveRun::new(req, fail_fast));
                        continue;
                    }

                    match rscheduler.recv() {
                        Ok(WorkerCommand::Run(req)) => {
                            on_event(WorkerEvent::Started {
                                run_id: req.run_id,
                                trigger: req.trigger.clone(),
                                commands: req.commands.clone(),
                            });
                            active = Some(ActiveRun::new(req, fail_fast));
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
                                .and_then(|run| run.current_task.clone())
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
                        let elapsed = finished.started.elapsed();
                        let failures = finished
                            .results
                            .iter()
                            .filter_map(|result| result.as_ref().err().cloned())
                            .collect();
                        stdout::present_results(finished.results, elapsed);
                        on_event(WorkerEvent::Finished { elapsed, failures });
                    }
                }
            }

            stdout::info("Consumer thread finished.");
        });

        Worker {
            scheduler: Some(tscheduler),
            next_run_id: AtomicU64::new(0),
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
            let current_dir = std::env::current_dir().unwrap();
            let commands = rules::template(
                rules::commands(rules),
                rules::TemplateOptions {
                    filepath: filepath.map(str::to_string),
                    current_dir: format!("{}", current_dir.display()),
                },
            );
            let run_id = self.next_run_id.fetch_add(1, Ordering::Relaxed) + 1;
            let request = RunRequest {
                run_id,
                commands,
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
