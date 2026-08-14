//! Shared task execution engine (TASK-0026).
//!
//! One executor owns process spawn, polling, fail-fast, cancellation, outcome
//! collection, timing, and lifecycle events. Wait and restart policies only
//! decide how plans are submitted or replaced.

use crate::cmd::{self, LoggedChild};
use crate::rules::CommandLine;
use crate::stdout;
use std::collections::VecDeque;
use std::sync::Arc;
use std::time::{Duration, Instant};

const POLL_INTERVAL: Duration = Duration::from_millis(10);

#[derive(Clone, Debug)]
pub enum Event {
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
    Tick {
        task: String,
    },
}

pub trait EventSink: Send + Sync {
    fn emit(&self, event: Event);
}

impl<F> EventSink for F
where
    F: Fn(Event) + Send + Sync,
{
    fn emit(&self, event: Event) {
        self(event);
    }
}

pub trait ProcessRunner: Send + Sync {
    fn spawn(&self, command: &CommandLine) -> Result<LoggedChild, String>;
}

pub struct SystemProcessRunner;

impl ProcessRunner for SystemProcessRunner {
    fn spawn(&self, command: &CommandLine) -> Result<LoggedChild, String> {
        match command {
            CommandLine::Shell(command) => cmd::spawn(command),
            CommandLine::Argv(argv) => cmd::spawn_argv(argv),
        }
    }
}

pub trait Clock: Send + Sync {
    fn now(&self) -> Instant;
    fn elapsed(&self, started: Instant) -> Duration;
    fn sleep(&self, duration: Duration);
}

pub struct SystemClock;

impl Clock for SystemClock {
    fn now(&self) -> Instant {
        Instant::now()
    }

    fn elapsed(&self, started: Instant) -> Duration {
        started.elapsed()
    }

    fn sleep(&self, duration: Duration) {
        std::thread::sleep(duration);
    }
}

#[derive(Clone, Debug)]
pub struct RunMetadata {
    pub run_id: u64,
    pub trigger: String,
}

impl RunMetadata {
    pub fn new(run_id: u64, trigger: impl Into<String>) -> Self {
        Self {
            run_id,
            trigger: trigger.into(),
        }
    }
}

pub struct CompletedRun {
    pub results: Vec<Result<(), String>>,
    pub elapsed: Duration,
}

pub enum Step {
    Running,
    Finished,
}

pub struct Run {
    commands: VecDeque<CommandLine>,
    results: Vec<Result<(), String>>,
    started: Instant,
    child: Option<LoggedChild>,
    current_task: Option<String>,
}

pub struct Executor {
    runner: Arc<dyn ProcessRunner>,
    clock: Arc<dyn Clock>,
    concurrency_limit: usize,
    events: Arc<dyn EventSink>,
    fail_fast: bool,
    verbose: bool,
}

impl Executor {
    pub fn new(
        runner: Arc<dyn ProcessRunner>,
        clock: Arc<dyn Clock>,
        concurrency_limit: usize,
        events: Arc<dyn EventSink>,
        fail_fast: bool,
        verbose: bool,
    ) -> Result<Self, String> {
        if concurrency_limit != 1 {
            return Err(format!(
                "executor currently supports concurrency limit 1, got {}",
                concurrency_limit
            ));
        }

        Ok(Self {
            runner,
            clock,
            concurrency_limit,
            events,
            fail_fast,
            verbose,
        })
    }

    pub fn concurrency_limit(&self) -> usize {
        self.concurrency_limit
    }

    pub fn start(&self, metadata: RunMetadata, commands: Vec<CommandLine>) -> Run {
        self.events.emit(Event::Started {
            run_id: metadata.run_id,
            trigger: metadata.trigger,
            commands: commands.iter().map(CommandLine::display).collect(),
        });

        Run {
            commands: commands.into(),
            results: vec![],
            started: self.clock.now(),
            child: None,
            current_task: None,
        }
    }

    pub fn advance(&self, run: &mut Run) -> Step {
        loop {
            if run.child.is_none() {
                let Some(task) = run.commands.pop_front() else {
                    return Step::Finished;
                };
                let display = task.display();
                run.current_task = Some(display.clone());
                match self.runner.spawn(&task) {
                    Ok(child) => {
                        run.child = Some(child);
                        self.events.emit(Event::Tick { task: display });
                        return Step::Running;
                    }
                    Err(err) => {
                        let failure = format!("Command {} failed to start: {}", display, err);
                        stdout::error(&failure);
                        run.results.push(Err(failure));
                        run.current_task = None;
                        if self.fail_fast {
                            return Step::Finished;
                        }
                    }
                }
                continue;
            }

            let task = run.current_task.clone().unwrap_or_default();
            match run.child.as_mut().expect("child is running").try_wait() {
                Ok(None) => {
                    self.events.emit(Event::Tick { task });
                    return Step::Running;
                }
                Ok(Some(status)) => {
                    run.child = None;
                    run.current_task = None;
                    if status.success() {
                        run.results.push(Ok(()));
                        continue;
                    }

                    run.results
                        .push(Err(format!("Command {} has failed with {}", task, status)));
                    if self.fail_fast {
                        return Step::Finished;
                    }
                }
                Err(err) => {
                    run.child = None;
                    run.current_task = None;
                    run.results
                        .push(Err(format!("Command {} has errored with {}", task, err)));
                    if self.fail_fast {
                        return Step::Finished;
                    }
                }
            }
        }
    }

    pub fn finish(&self, run: Run) -> CompletedRun {
        let elapsed = self.clock.elapsed(run.started);
        let failures = run
            .results
            .iter()
            .filter_map(|result| result.as_ref().err().cloned())
            .collect();
        self.events.emit(Event::Finished { elapsed, failures });
        CompletedRun {
            results: run.results,
            elapsed,
        }
    }

    pub fn cancel(&self, run: &mut Run) {
        if let Some(child) = run.child.as_mut() {
            let task = run.current_task.clone().unwrap_or_default();
            let (signal, grace) = crate::process_owner::shutdown_policy();
            stdout::verbose(&format!("---- cancelling: {:?} ----", task), self.verbose);
            let outcome = child.shutdown(signal, grace, self.verbose);
            let report = match outcome {
                crate::cmd::ShutdownOutcome::AlreadyExited(status)
                | crate::cmd::ShutdownOutcome::Terminated(status) => {
                    format!("---- finished: {:?} status: {} ----", task, status)
                }
                crate::cmd::ShutdownOutcome::Escalated { status } => {
                    let detail = status
                        .map(|status| status.to_string())
                        .unwrap_or_else(|| "(reap failed)".to_owned());
                    format!("---- force-killed: {:?} status: {} ----", task, detail)
                }
            };
            stdout::verbose(&report, self.verbose);
        }
        run.child = None;
        run.commands.clear();
        self.events.emit(Event::Cancelled);
    }

    pub fn run_to_completion(
        &self,
        metadata: RunMetadata,
        commands: Vec<CommandLine>,
    ) -> CompletedRun {
        let mut run = self.start(metadata, commands);
        loop {
            match self.advance(&mut run) {
                Step::Finished => return self.finish(run),
                Step::Running => self.clock.sleep(POLL_INTERVAL),
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::{Arc, Mutex};

    fn shell(cmd: &str) -> CommandLine {
        CommandLine::Shell(cmd.to_owned())
    }

    fn run_commands(commands: Vec<CommandLine>, fail_fast: bool) -> Vec<Result<(), String>> {
        Executor::new(
            Arc::new(SystemProcessRunner),
            Arc::new(SystemClock),
            1,
            Arc::new(|_| {}),
            fail_fast,
            false,
        )
        .expect("concurrency one is supported")
        .run_to_completion(RunMetadata::new(0, "test"), commands)
        .results
    }

    #[test]
    fn runs_all_commands_when_all_pass() {
        let results = run_commands(vec![shell("true"), shell("true")], true);
        assert_eq!(results.len(), 2);
        assert!(results.iter().all(|result| result.is_ok()));
    }

    #[test]
    fn fail_fast_stops_at_first_failure() {
        let results = run_commands(vec![shell("false"), shell("true")], true);
        assert_eq!(results.len(), 1, "fail-fast must not run later commands");
        assert!(results[0].is_err());
    }

    #[test]
    fn without_fail_fast_later_commands_still_run_after_a_failure() {
        let results = run_commands(vec![shell("false"), shell("true")], false);
        assert_eq!(results.len(), 2, "non-fail-fast must run every command");
        assert!(results[0].is_err());
        assert!(results[1].is_ok());
    }

    #[test]
    fn empty_command_list_produces_no_results() {
        assert!(run_commands(vec![], true).is_empty());
        assert!(run_commands(vec![], false).is_empty());
    }

    #[test]
    fn argv_commands_run_directly_without_shell() {
        let argv = CommandLine::Argv(vec![
            "sh".to_owned(),
            "-c".to_owned(),
            "test \"$#\" = 2".to_owned(),
            "probe".to_owned(),
            "a b".to_owned(),
            "c".to_owned(),
        ]);
        let results = run_commands(vec![argv], true);
        assert_eq!(results.len(), 1);
        assert!(results[0].is_ok(), "argv boundary preserved: {:?}", results);
    }

    #[test]
    fn shell_and_argv_dispatch_through_one_runner() {
        let results = run_commands(
            vec![shell("true"), CommandLine::Argv(vec!["true".to_owned()])],
            true,
        );
        assert_eq!(results.len(), 2);
        assert!(results.iter().all(|result| result.is_ok()));
    }

    #[derive(Default)]
    struct RecordingRunner {
        commands: Mutex<Vec<String>>,
    }

    impl ProcessRunner for RecordingRunner {
        fn spawn(&self, command: &CommandLine) -> Result<LoggedChild, String> {
            self.commands.lock().unwrap().push(command.display());
            SystemProcessRunner.spawn(command)
        }
    }

    struct FixedClock;

    impl Clock for FixedClock {
        fn now(&self) -> Instant {
            Instant::now()
        }

        fn elapsed(&self, _started: Instant) -> Duration {
            Duration::from_millis(42)
        }

        fn sleep(&self, _duration: Duration) {}
    }

    #[test]
    fn executor_receives_runner_clock_limit_and_event_sink_explicitly() {
        let runner = Arc::new(RecordingRunner::default());
        let events = Arc::new(Mutex::new(Vec::new()));
        let recorded_events = Arc::clone(&events);
        let executor = Executor::new(
            runner.clone(),
            Arc::new(FixedClock),
            1,
            Arc::new(move |event| recorded_events.lock().unwrap().push(event)),
            true,
            false,
        )
        .expect("concurrency one is supported");
        assert_eq!(executor.concurrency_limit(), 1);

        let completed =
            executor.run_to_completion(RunMetadata::new(7, "test"), vec![shell("true")]);

        assert_eq!(completed.elapsed, Duration::from_millis(42));
        assert_eq!(*runner.commands.lock().unwrap(), vec!["true"]);
        let events = events.lock().unwrap();
        assert!(matches!(
            events.first(),
            Some(Event::Started { run_id: 7, trigger, .. }) if trigger == "test"
        ));
        assert!(matches!(
            events.last(),
            Some(Event::Finished { elapsed, failures })
                if *elapsed == Duration::from_millis(42) && failures.is_empty()
        ));
    }

    #[test]
    fn executor_rejects_unsupported_concurrency_limits() {
        for limit in [0, 2] {
            let result = Executor::new(
                Arc::new(SystemProcessRunner),
                Arc::new(SystemClock),
                limit,
                Arc::new(|_| {}),
                true,
                false,
            );
            assert!(result.is_err(), "limit {limit} must be rejected");
        }
    }
}
