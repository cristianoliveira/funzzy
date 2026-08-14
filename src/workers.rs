use crate::executor::{Event, Executor, Run, RunMetadata, Step, SystemClock, SystemProcessRunner};
use crate::plan::RunPlan;
use crate::rules::Rules;
use crate::stdout;
use crate::template::TemplateOptions;
use std::path::PathBuf;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, Condvar, Mutex};
use std::thread::JoinHandle;
use std::time::Duration;

/// A run requested through the worker's command stream.
struct RunRequest {
    run_id: u64,
    plan: RunPlan,
    trigger: String,
}

/// Scheduling and replacement flow through one ordered stream, so a newer run
/// always supersedes any queued work instead of racing a separate cancel
/// channel.
enum WorkerCommand {
    Run(RunRequest),
    Cancel,
}

/// A single overwrite slot: bounded state that always retains only the newest
/// scheduling decision. The condition variable blocks idle consumers without
/// polling or growing an unbounded command queue.
#[derive(Default)]
struct SchedulerState {
    pending: Option<WorkerCommand>,
    closed: bool,
}

#[derive(Default)]
struct Scheduler {
    state: Mutex<SchedulerState>,
    ready: Condvar,
}

impl Scheduler {
    fn send(&self, command: WorkerCommand) {
        self.state.lock().unwrap().pending = Some(command);
        self.ready.notify_one();
    }

    fn take_newest(&self) -> Option<WorkerCommand> {
        self.state.lock().unwrap().pending.take()
    }

    fn receive(&self) -> Option<WorkerCommand> {
        let mut state = self.state.lock().unwrap();
        while state.pending.is_none() && !state.closed {
            state = self.ready.wait(state).unwrap();
        }
        state.pending.take()
    }

    fn close(&self) {
        self.state.lock().unwrap().closed = true;
        self.ready.notify_all();
    }
}

pub struct Worker {
    scheduler: Option<Arc<Scheduler>>,
    next_run_id: AtomicU64,
    root: PathBuf,
    verbose: bool,

    consumer: Option<JoinHandle<()>>,
}

impl Worker {
    /// Convenience constructor resolving the workspace root from the process
    /// current directory. Keep usage at the outer boundary; prefer
    /// [`Worker::with_root`] so command template preparation does not depend
    /// on hidden process state.
    pub fn new<F>(verbose: bool, fail_fast: bool, on_event: F) -> Self
    where
        F: Fn(Event) + Send + Sync + 'static,
    {
        let root = std::env::current_dir().expect("Unable to get current directory");
        Self::with_root(verbose, fail_fast, root, on_event)
    }

    /// Creates a worker that expands command templates against an explicit
    /// workspace root and the host's available parallelism.
    pub fn with_root<F>(verbose: bool, fail_fast: bool, root: PathBuf, on_event: F) -> Self
    where
        F: Fn(Event) + Send + Sync + 'static,
    {
        let jobs = std::thread::available_parallelism()
            .map(|jobs| jobs.get())
            .unwrap_or(1);
        Self::with_root_and_jobs(verbose, fail_fast, root, jobs, on_event)
    }

    /// Creates a worker with an explicit task-concurrency bound.
    pub fn with_root_and_jobs<F>(
        verbose: bool,
        fail_fast: bool,
        root: PathBuf,
        jobs: usize,
        on_event: F,
    ) -> Self
    where
        F: Fn(Event) + Send + Sync + 'static,
    {
        stdout::verbose("Worker in verbose mode.", verbose);
        let scheduler = Arc::new(Scheduler::default());
        let consumer_scheduler = Arc::clone(&scheduler);
        let events = Arc::new(move |event: Event| {
            if let Event::Tick { task } = &event {
                stdout::verbose(&format!("waiting next tick for task: {}", task), verbose);
            }
            on_event(event);
        });
        let executor = Executor::new(
            Arc::new(SystemProcessRunner),
            Arc::new(SystemClock),
            jobs,
            events,
            fail_fast,
            verbose,
        )
        .expect("worker jobs must be positive");

        let consumer = std::thread::spawn(move || {
            let mut active: Option<Run> = None;
            let mut pending: Option<RunRequest> = None;

            loop {
                if active.is_none() {
                    // Promote the newest superseding run, or block on the next
                    // command when idle.
                    if let Some(req) = pending.take() {
                        active = Some(
                            executor.start(RunMetadata::new(req.run_id, req.trigger), req.plan),
                        );
                        continue;
                    }

                    match consumer_scheduler.receive() {
                        Some(WorkerCommand::Run(req)) => {
                            active = Some(
                                executor.start(RunMetadata::new(req.run_id, req.trigger), req.plan),
                            );
                        }
                        Some(WorkerCommand::Cancel) => {}
                        None => break,
                    }
                    continue;
                }

                let step = executor.advance(active.as_mut().expect("active run"));
                match step {
                    Step::Running => match consumer_scheduler.take_newest() {
                        Some(WorkerCommand::Run(req)) => {
                            let mut replaced = active.take().expect("active run");
                            executor.cancel(&mut replaced);
                            pending = Some(req);
                        }
                        Some(WorkerCommand::Cancel) => {
                            let mut cancelled = active.take().expect("active run");
                            executor.cancel(&mut cancelled);
                        }
                        None => std::thread::sleep(Duration::from_millis(200)),
                    },
                    Step::Finished => {
                        let completed = executor.finish(active.take().expect("active run"));
                        stdout::present_results(completed.results, completed.elapsed);
                    }
                }
            }

            stdout::info("Consumer thread finished.");
        });

        Worker {
            scheduler: Some(scheduler),
            next_run_id: AtomicU64::new(0),
            root,
            verbose,
            consumer: Some(consumer),
        }
    }

    pub fn cancel_running_tasks(&self) -> Result<(), String> {
        if let Some(scheduler) = self.scheduler.as_ref() {
            scheduler.send(WorkerCommand::Cancel);
        }
        Ok(())
    }

    pub fn schedule(&self, rules: Vec<Rules>, filepath: &str) -> Result<u64, String> {
        self.schedule_plan(RunPlan::from_rules(rules), filepath)
    }

    pub fn schedule_plan(&self, plan: RunPlan, filepath: &str) -> Result<u64, String> {
        self.schedule_plan_with_trigger(plan, filepath, Some(filepath))
    }

    #[cfg(test)]
    pub(crate) fn schedule_with_trigger(
        &self,
        rules: Vec<Rules>,
        trigger: &str,
        filepath: Option<&str>,
    ) -> Result<u64, String> {
        self.schedule_plan_with_trigger(RunPlan::from_rules(rules), trigger, filepath)
    }

    pub(crate) fn schedule_plan_with_trigger(
        &self,
        plan: RunPlan,
        trigger: &str,
        filepath: Option<&str>,
    ) -> Result<u64, String> {
        if let Some(scheduler) = self.scheduler.as_ref() {
            let plan = plan.resolve_context(&self.root)?;
            let (plan, unknown_variables) = plan.expand(&TemplateOptions {
                filepath: filepath.map(str::to_string),
                current_dir: format!("{}", self.root.display()),
            });
            stdout::verbose(&plan.context_summary(), self.verbose);
            for variable in unknown_variables {
                stdout::warn(&format!("Unknown template variable '{}'.", variable));
            }
            let run_id = self.next_run_id.fetch_add(1, Ordering::Relaxed) + 1;
            let request = RunRequest {
                run_id,
                plan,
                trigger: trigger.to_string(),
            };
            scheduler.send(WorkerCommand::Run(request));
            return Ok(run_id);
        }

        Err("worker scheduler is unavailable".to_string())
    }
}

impl Drop for Worker {
    fn drop(&mut self) {
        if let Some(scheduler) = self.scheduler.as_ref() {
            scheduler.send(WorkerCommand::Cancel);
            scheduler.close();
        }
        self.scheduler.take();
        if let Some(th) = self.consumer.take() {
            th.join().expect("failed to join consumer thread");
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::executor::Event as WorkerEvent;
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
        expect_event(&rx, "worker tick", |e| {
            matches!(e, WorkerEvent::Tick { .. })
        });

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
        std::fs::create_dir_all(&root).expect("create injected root");
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
        let _ = std::fs::remove_dir_all(&root);
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
