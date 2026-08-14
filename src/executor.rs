//! Bounded task execution engine (TASK-0026/TASK-0027).
//!
//! One executor owns process spawn, polling, fail-fast, cancellation, outcome
//! collection, timing, lifecycle events, and stage barriers. Wait and restart
//! policies only decide how plans are submitted or replaced.

use crate::cmd::{self, CaptureHandle, LoggedChild, ShutdownOutcome};
use crate::output::OutputRegistry;
use crate::plan::{RunOutcome, RunPlan, Stage, TaskContext, TaskOutcome, TaskPlan};
use crate::rules::CommandLine;
use crate::stdout;
use serde_derive::Serialize;
use std::collections::VecDeque;
use std::io;
use std::process::ExitStatus;
use std::sync::Arc;
use std::time::{Duration, Instant};

const POLL_INTERVAL: Duration = Duration::from_millis(10);

/// Wire-level task state for the correlated snapshot (contract §7). `Skipped`
/// (fail-fast skipped work) collapses to `Cancelled` — never-started work is
/// reported as cancelled, matching the pi-watcher decoder vocabulary.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize)]
#[serde(rename_all = "lowercase")]
pub enum TaskState {
    Passed,
    Failed,
    Cancelled,
}

/// One task's terminal outcome for the correlated snapshot (TASK-0050).
#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct TaskSnapshot {
    pub id: String,
    pub name: String,
    pub state: TaskState,
    pub duration_ms: Option<u64>,
}

#[derive(Clone, Debug)]
pub enum Event {
    Started {
        run_id: u64,
        trigger: String,
        /// Debounce batch identity (contract §1), when this generation was
        /// scheduled from a filesystem batch. None for init, target runs, and
        /// synthetic emits without a debounce batch.
        batch: Option<u64>,
        /// Generation identity this run replaces, when known at start.
        predecessor: Option<u64>,
        /// Complete normalized changed-path set of the triggering batch.
        changed: Vec<String>,
        commands: Vec<String>,
    },
    Finished {
        run_id: u64,
        /// Generation identity that superseded this one, when replaced.
        superseded_by: Option<u64>,
        elapsed: Duration,
        failures: Vec<String>,
    },
    Cancelled {
        run_id: u64,
        /// Generation identity that superseded this one, when replaced.
        superseded_by: Option<u64>,
    },
    Tick {
        task: String,
        group_occurrence: Option<String>,
    },
    /// One task reached a terminal outcome (passed/failed/cancelled) within a
    /// generation. Emitted per task so the correlated snapshot can show task
    /// outcomes and durations (TASK-0050).
    TaskTerminal { run_id: u64, task: TaskSnapshot },
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

pub trait ChildProcess: Send {
    fn try_wait(&mut self) -> io::Result<Option<ExitStatus>>;
    fn shutdown(
        &mut self,
        signal: nix::sys::signal::Signal,
        grace: Duration,
        verbose: bool,
    ) -> ShutdownOutcome;
}

impl ChildProcess for LoggedChild {
    fn try_wait(&mut self) -> io::Result<Option<ExitStatus>> {
        LoggedChild::try_wait(self)
    }

    fn shutdown(
        &mut self,
        signal: nix::sys::signal::Signal,
        grace: Duration,
        verbose: bool,
    ) -> ShutdownOutcome {
        LoggedChild::shutdown(self, signal, grace, verbose)
    }
}

pub trait ProcessRunner: Send + Sync {
    fn spawn(
        &self,
        task: &str,
        command: &CommandLine,
        context: &TaskContext,
        capture: Option<Arc<CaptureHandle>>,
    ) -> Result<Box<dyn ChildProcess>, String>;
}

pub struct SystemProcessRunner;

impl ProcessRunner for SystemProcessRunner {
    fn spawn(
        &self,
        _task: &str,
        command: &CommandLine,
        context: &TaskContext,
        capture: Option<Arc<CaptureHandle>>,
    ) -> Result<Box<dyn ChildProcess>, String> {
        let child = match command {
            CommandLine::Shell(command) => cmd::spawn_in_with_capture(command, context, capture),
            CommandLine::Argv(argv) => cmd::spawn_argv_in(argv, context),
        }?;
        Ok(Box::new(child))
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
    /// Debounce batch identity when this run was scheduled from a batch.
    pub batch: Option<u64>,
    /// Generation identity this run replaces (restart policy).
    pub predecessor: Option<u64>,
    /// Generation identity that replaced this one; filled at cancellation.
    pub superseded_by: Option<u64>,
    /// Complete normalized changed-path set of the triggering batch.
    pub changed: Vec<String>,
}

impl RunMetadata {
    pub fn new(run_id: u64, trigger: impl Into<String>) -> Self {
        Self {
            run_id,
            trigger: trigger.into(),
            batch: None,
            predecessor: None,
            superseded_by: None,
            changed: vec![],
        }
    }

    /// Builds metadata for a generation scheduled from a debounce batch,
    /// retaining the batch identity and complete changed-path set.
    pub fn correlated(
        run_id: u64,
        trigger: impl Into<String>,
        batch: Option<u64>,
        predecessor: Option<u64>,
        changed: Vec<String>,
    ) -> Self {
        Self {
            run_id,
            trigger: trigger.into(),
            batch,
            predecessor,
            superseded_by: None,
            changed,
        }
    }
}

pub struct CompletedRun {
    pub results: Vec<Result<(), String>>,
    pub elapsed: Duration,
    pub outcome: RunOutcome,
}

/// How a run cancellation ended (TASK-0046): graceful when every active child
/// terminated after the initial signal within grace, escalated when at least
/// one child ignored the graceful signal and was force-killed.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum CancelDisposition {
    Graceful,
    Escalated,
}

pub enum Step {
    Running,
    Finished,
}

struct ActiveTask {
    name: String,
    position: usize,
    commands: VecDeque<CommandLine>,
    child: Option<Box<dyn ChildProcess>>,
    current_command: Option<String>,
    failures: Vec<String>,
    context: TaskContext,
    context_validated: bool,
    group_occurrence: Option<String>,
    /// When the first command of this task spawned, for per-task duration
    /// (TASK-0050). None until a command actually starts.
    started: Option<Instant>,
    /// Bounded per-stream output capture for this task, when the executor
    /// feeds a retention registry (TASK-0045).
    capture: Option<Arc<CaptureHandle>>,
}

impl From<TaskPlan> for ActiveTask {
    fn from(task: TaskPlan) -> Self {
        Self {
            name: task.name,
            position: task.position,
            commands: task.commands.into(),
            child: None,
            current_command: None,
            failures: vec![],
            context: task.context,
            context_validated: false,
            group_occurrence: task.group_occurrence,
            started: None,
            capture: None,
        }
    }
}

pub struct Run {
    stages: VecDeque<Stage>,
    queued: VecDeque<TaskPlan>,
    active: Vec<ActiveTask>,
    stage_limit: usize,
    results: Vec<Result<(), String>>,
    outcomes: Vec<(usize, String, TaskOutcome)>,
    metadata: RunMetadata,
    superseded_by: Option<u64>,
    started: Instant,
}

impl Run {
    /// The generation identity of this run.
    pub fn run_id(&self) -> u64 {
        self.metadata.run_id
    }
}

enum TaskStep {
    Running,
    Finished,
    FailedFast,
}

pub struct Executor {
    runner: Arc<dyn ProcessRunner>,
    clock: Arc<dyn Clock>,
    concurrency_limit: usize,
    events: Arc<dyn EventSink>,
    fail_fast: bool,
    verbose: bool,
    /// Retained-output registry fed at task terminal (TASK-0045); None keeps
    /// capture disabled (no control surface consumes it).
    outputs: Option<Arc<OutputRegistry>>,
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
        if concurrency_limit == 0 {
            return Err("executor concurrency limit must be positive".to_owned());
        }

        Ok(Self {
            runner,
            clock,
            concurrency_limit,
            events,
            fail_fast,
            verbose,
            outputs: None,
        })
    }

    /// Like [`Executor::new`], additionally feeding a retained-output
    /// registry (TASK-0045): each task's stdout/stderr is captured bounded
    /// and recorded for the generation when the task terminates.
    pub fn with_outputs(
        runner: Arc<dyn ProcessRunner>,
        clock: Arc<dyn Clock>,
        concurrency_limit: usize,
        events: Arc<dyn EventSink>,
        fail_fast: bool,
        verbose: bool,
        outputs: Option<Arc<OutputRegistry>>,
    ) -> Result<Self, String> {
        if concurrency_limit == 0 {
            return Err("executor concurrency limit must be positive".to_owned());
        }

        Ok(Self {
            runner,
            clock,
            concurrency_limit,
            events,
            fail_fast,
            verbose,
            outputs,
        })
    }

    pub fn concurrency_limit(&self) -> usize {
        self.concurrency_limit
    }

    pub fn start(&self, metadata: RunMetadata, plan: RunPlan) -> Run {
        self.events.emit(Event::Started {
            run_id: metadata.run_id,
            trigger: metadata.trigger.clone(),
            batch: metadata.batch,
            predecessor: metadata.predecessor,
            changed: metadata.changed.clone(),
            commands: plan.commands().iter().map(CommandLine::display).collect(),
        });

        Run {
            stages: plan.stages.into(),
            queued: VecDeque::new(),
            active: vec![],
            stage_limit: 0,
            results: vec![],
            outcomes: vec![],
            metadata,
            superseded_by: None,
            started: self.clock.now(),
        }
    }

    pub fn advance(&self, run: &mut Run) -> Step {
        loop {
            if run.active.is_empty() && run.queued.is_empty() {
                let Some(stage) = run.stages.pop_front() else {
                    return Step::Finished;
                };
                match stage {
                    Stage::Serial(task) => {
                        run.stage_limit = 1;
                        run.queued.push_back(task);
                    }
                    Stage::Parallel { tasks, .. } => {
                        run.stage_limit = self.concurrency_limit.min(tasks.len());
                        run.queued.extend(tasks);
                    }
                }
            }

            self.fill_available_slots(run);
            let mut task_finished = false;
            let mut index = 0;
            while index < run.active.len() {
                match self.advance_task(&mut run.active[index], &mut run.results) {
                    TaskStep::Running => index += 1,
                    TaskStep::Finished => {
                        let task = run.active.remove(index);
                        self.record_task_outcome(run, task);
                        task_finished = true;
                    }
                    TaskStep::FailedFast => {
                        let task = run.active.remove(index);
                        self.record_task_outcome(run, task);
                        self.stop_after_failure(run);
                        return Step::Finished;
                    }
                }
            }

            if run.active.is_empty() && run.queued.is_empty() {
                continue;
            }

            if task_finished && run.active.len() < run.stage_limit && !run.queued.is_empty() {
                continue;
            }

            return Step::Running;
        }
    }

    fn fill_available_slots(&self, run: &mut Run) {
        while run.active.len() < run.stage_limit {
            let Some(task) = run.queued.pop_front() else {
                return;
            };
            run.active.push(task.into());
        }
    }

    fn advance_task(
        &self,
        task: &mut ActiveTask,
        results: &mut Vec<Result<(), String>>,
    ) -> TaskStep {
        if !task.context_validated {
            task.context_validated = true;
            if let Some(cwd) = &task.context.cwd {
                if !cwd.is_dir() {
                    let failure = format!(
                        "Task '{}' cwd is missing or not a directory: {}",
                        task.name,
                        cwd.display()
                    );
                    task.failures.push(failure.clone());
                    results.push(Err(failure));
                    task.commands.clear();
                    return if self.fail_fast {
                        TaskStep::FailedFast
                    } else {
                        TaskStep::Finished
                    };
                }
            }
        }

        loop {
            if task.child.is_none() {
                let Some(command) = task.commands.pop_front() else {
                    return TaskStep::Finished;
                };
                let display = command.display();
                task.current_command = Some(display.clone());
                if task.capture.is_none() && self.outputs.is_some() {
                    task.capture = Some(Arc::new(CaptureHandle::new()));
                }
                match self
                    .runner
                    .spawn(&task.name, &command, &task.context, task.capture.clone())
                {
                    Ok(child) => {
                        task.child = Some(child);
                        if task.started.is_none() {
                            task.started = Some(self.clock.now());
                        }
                        self.events.emit(Event::Tick {
                            task: task.name.clone(),
                            group_occurrence: task.group_occurrence.clone(),
                        });
                        return TaskStep::Running;
                    }
                    Err(err) => {
                        let failure = format!("Command {} failed to start: {}", display, err);
                        stdout::error(&failure);
                        task.failures.push(failure.clone());
                        results.push(Err(failure));
                        task.current_command = None;
                        if self.fail_fast {
                            return TaskStep::FailedFast;
                        }
                    }
                }
                continue;
            }

            let display = task.current_command.clone().unwrap_or_default();
            match task.child.as_mut().expect("child is running").try_wait() {
                Ok(None) => {
                    self.events.emit(Event::Tick {
                        task: task.name.clone(),
                        group_occurrence: task.group_occurrence.clone(),
                    });
                    return TaskStep::Running;
                }
                Ok(Some(status)) => {
                    task.child = None;
                    task.current_command = None;
                    if status.success() {
                        results.push(Ok(()));
                        continue;
                    }

                    let failure = format!("Command {} has failed with {}", display, status);
                    task.failures.push(failure.clone());
                    results.push(Err(failure));
                    if self.fail_fast {
                        return TaskStep::FailedFast;
                    }
                }
                Err(err) => {
                    task.child = None;
                    task.current_command = None;
                    let failure = format!("Command {} has errored with {}", display, err);
                    task.failures.push(failure.clone());
                    results.push(Err(failure));
                    if self.fail_fast {
                        return TaskStep::FailedFast;
                    }
                }
            }
        }
    }

    /// Emits one per-task terminal record for the correlated snapshot
    /// (TASK-0050). The id is the stable group-occurrence identity when a task
    /// belongs to a named parallel group, else the task name.
    fn record_task_snapshot(
        &self,
        run_id: u64,
        name: &str,
        group_occurrence: Option<&str>,
        state: TaskState,
        duration_ms: Option<u64>,
    ) {
        self.events.emit(Event::TaskTerminal {
            run_id,
            task: TaskSnapshot {
                id: group_occurrence
                    .map(str::to_owned)
                    .unwrap_or_else(|| name.to_owned()),
                name: name.to_owned(),
                state,
                duration_ms,
            },
        });
    }

    fn record_task_outcome(&self, run: &mut Run, task: ActiveTask) {
        if let (Some(outputs), Some(capture)) = (&self.outputs, &task.capture) {
            outputs.record(run.metadata.run_id, task.name.clone(), capture.finish());
        }
        let duration_ms = task
            .started
            .map(|started| self.clock.elapsed(started).as_millis() as u64);
        let (state, outcome) = if task.failures.is_empty() {
            (TaskState::Passed, TaskOutcome::Passed)
        } else {
            let failures = task.failures.clone();
            (TaskState::Failed, TaskOutcome::Failed { failures })
        };
        self.record_task_snapshot(
            run.metadata.run_id,
            &task.name,
            task.group_occurrence.as_deref(),
            state,
            duration_ms,
        );
        run.outcomes.push((task.position, task.name, outcome));
    }

    fn stop_after_failure(&self, run: &mut Run) {
        for mut task in run.active.drain(..) {
            self.shutdown_task(&mut task);
            if let (Some(outputs), Some(capture)) = (&self.outputs, &task.capture) {
                outputs.record(run.metadata.run_id, task.name.clone(), capture.finish());
            }
            let duration_ms = task
                .started
                .map(|started| self.clock.elapsed(started).as_millis() as u64);
            self.record_task_snapshot(
                run.metadata.run_id,
                &task.name,
                task.group_occurrence.as_deref(),
                TaskState::Cancelled,
                duration_ms,
            );
            run.outcomes
                .push((task.position, task.name, TaskOutcome::Cancelled));
        }
        for task in run.queued.drain(..) {
            self.record_task_snapshot(
                run.metadata.run_id,
                &task.name,
                task.group_occurrence.as_deref(),
                TaskState::Cancelled,
                None,
            );
            run.outcomes
                .push((task.position, task.name, TaskOutcome::Skipped));
        }
        for stage in run.stages.drain(..) {
            for task in stage_tasks(stage) {
                self.record_task_snapshot(
                    run.metadata.run_id,
                    &task.name,
                    task.group_occurrence.as_deref(),
                    TaskState::Cancelled,
                    None,
                );
                run.outcomes
                    .push((task.position, task.name, TaskOutcome::Skipped));
            }
        }
    }

    pub fn finish(&self, mut run: Run) -> CompletedRun {
        let elapsed = self.clock.elapsed(run.started);
        run.outcomes.sort_by_key(|(position, _, _)| *position);
        let outcome = RunOutcome::from_task_outcomes(
            run.outcomes
                .into_iter()
                .map(|(_, name, outcome)| (name, outcome))
                .collect(),
        );
        let failures = outcome.failures();
        self.events.emit(Event::Finished {
            run_id: run.metadata.run_id,
            superseded_by: run.superseded_by,
            elapsed,
            failures: failures.clone(),
        });
        CompletedRun {
            results: run.results,
            elapsed,
            outcome,
        }
    }

    /// Cancels the run and records which generation superseded it, when any.
    /// The replacement relation (contract §1) is carried on the Cancelled
    /// event so superseded generations are never reported as passed/failed.
    /// Cancels the run and records which generation superseded it, when any.
    /// Returns the disposition: graceful when every active child terminated
    /// after the initial signal, escalated when any child was force-killed.
    pub fn cancel(&self, run: &mut Run, superseded_by: Option<u64>) -> CancelDisposition {
        run.superseded_by = superseded_by;
        let mut escalated = false;
        for task in &mut run.active {
            if self.shutdown_task(task) {
                escalated = true;
            }
            if let (Some(outputs), Some(capture)) = (&self.outputs, &task.capture) {
                outputs.record(run.metadata.run_id, task.name.clone(), capture.finish());
            }
            let duration_ms = task
                .started
                .map(|started| self.clock.elapsed(started).as_millis() as u64);
            self.record_task_snapshot(
                run.metadata.run_id,
                &task.name,
                task.group_occurrence.as_deref(),
                TaskState::Cancelled,
                duration_ms,
            );
        }
        for task in run.queued.drain(..) {
            self.record_task_snapshot(
                run.metadata.run_id,
                &task.name,
                task.group_occurrence.as_deref(),
                TaskState::Cancelled,
                None,
            );
        }
        for stage in run.stages.drain(..) {
            for task in stage_tasks(stage) {
                self.record_task_snapshot(
                    run.metadata.run_id,
                    &task.name,
                    task.group_occurrence.as_deref(),
                    TaskState::Cancelled,
                    None,
                );
            }
        }
        run.active.clear();
        run.queued.clear();
        run.stages.clear();
        self.events.emit(Event::Cancelled {
            run_id: run.metadata.run_id,
            superseded_by,
        });
        if escalated {
            CancelDisposition::Escalated
        } else {
            CancelDisposition::Graceful
        }
    }

    fn shutdown_task(&self, task: &mut ActiveTask) -> bool {
        let Some(child) = task.child.as_mut() else {
            task.commands.clear();
            return false;
        };
        let (signal, grace) = crate::process_owner::shutdown_policy();
        stdout::verbose(
            &format!("---- cancelling task: {:?} ----", task.name),
            self.verbose,
        );
        let outcome = child.shutdown(signal, grace, self.verbose);
        let escalated = matches!(outcome, ShutdownOutcome::Escalated { .. });
        let report = match outcome {
            ShutdownOutcome::AlreadyExited(status) | ShutdownOutcome::Terminated(status) => {
                format!(
                    "---- finished task: {:?} status: {} ----",
                    task.name, status
                )
            }
            ShutdownOutcome::Escalated { status } => {
                let detail = status
                    .map(|status| status.to_string())
                    .unwrap_or_else(|| "(reap failed)".to_owned());
                format!(
                    "---- force-killed task: {:?} status: {} ----",
                    task.name, detail
                )
            }
        };
        stdout::verbose(&report, self.verbose);
        task.child = None;
        task.commands.clear();
        escalated
    }

    pub fn run_to_completion(&self, metadata: RunMetadata, plan: RunPlan) -> CompletedRun {
        let mut run = self.start(metadata, plan);
        loop {
            match self.advance(&mut run) {
                Step::Finished => return self.finish(run),
                Step::Running => self.clock.sleep(POLL_INTERVAL),
            }
        }
    }
}

fn stage_tasks(stage: Stage) -> Vec<TaskPlan> {
    match stage {
        Stage::Serial(task) => vec![task],
        Stage::Parallel { tasks, .. } => tasks,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::rules::Rules;
    use std::collections::{HashMap, HashSet};
    use std::os::unix::process::ExitStatusExt;
    use std::sync::{Arc, Mutex};

    fn shell(command: &str) -> CommandLine {
        CommandLine::Shell(command.to_owned())
    }

    fn task(name: &str, group: Option<&str>, commands: &[&str]) -> Rules {
        let rule = Rules::new(
            name.to_owned(),
            commands.iter().map(|command| command.to_string()).collect(),
            vec!["src/**".to_owned()],
            vec![],
            false,
        );
        match group {
            Some(group) => rule.with_parallel(group.to_owned()),
            None => rule,
        }
    }

    fn run_commands(commands: Vec<CommandLine>, fail_fast: bool) -> Vec<Result<(), String>> {
        let rule = Rules::new(
            "test".to_owned(),
            vec![],
            vec!["src/**".to_owned()],
            vec![],
            false,
        );
        let plan = RunPlan {
            stages: vec![Stage::Serial(TaskPlan {
                name: "test".to_owned(),
                position: 0,
                commands,
                parallel: None,
                group_occurrence: None,
                rule,
                context: crate::plan::TaskContext::default(),
            })],
        };
        Executor::new(
            Arc::new(SystemProcessRunner),
            Arc::new(SystemClock),
            1,
            Arc::new(|_| {}),
            fail_fast,
            false,
        )
        .expect("concurrency one is supported")
        .run_to_completion(RunMetadata::new(0, "test"), plan)
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
        fn spawn(
            &self,
            task: &str,
            command: &CommandLine,
            context: &TaskContext,
            capture: Option<Arc<CaptureHandle>>,
        ) -> Result<Box<dyn ChildProcess>, String> {
            self.commands
                .lock()
                .unwrap()
                .push(format!("{}:{}", task, command.display()));
            SystemProcessRunner.spawn(task, command, context, capture)
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

        let completed = executor.run_to_completion(
            RunMetadata::new(7, "test"),
            RunPlan::from_rules(vec![task("task", None, &["true"])]),
        );

        assert_eq!(completed.elapsed, Duration::from_millis(42));
        assert_eq!(*runner.commands.lock().unwrap(), vec!["task:true"]);
        let events = events.lock().unwrap();
        assert!(matches!(
            events.first(),
            Some(Event::Started { run_id: 7, trigger, .. }) if trigger == "test"
        ));
        assert!(matches!(
            events.last(),
            Some(Event::Finished { elapsed, failures, .. })
                if *elapsed == Duration::from_millis(42) && failures.is_empty()
        ));
    }

    #[derive(Default)]
    struct FakeState {
        active: usize,
        max_active: usize,
        started: Vec<(String, String)>,
        completed: HashMap<String, bool>,
        shutdown: HashSet<String>,
    }

    #[derive(Clone, Default)]
    struct FakeRunner {
        state: Arc<Mutex<FakeState>>,
    }

    impl FakeRunner {
        fn complete(&self, command: &str, success: bool) {
            self.state
                .lock()
                .unwrap()
                .completed
                .insert(command.to_owned(), success);
        }

        fn started_commands(&self) -> Vec<String> {
            self.state
                .lock()
                .unwrap()
                .started
                .iter()
                .map(|(_, command)| command.clone())
                .collect()
        }
    }

    struct FakeChild {
        command: String,
        state: Arc<Mutex<FakeState>>,
        terminal: bool,
    }

    impl FakeChild {
        fn terminal_status(&mut self, success: bool) -> ExitStatus {
            if !self.terminal {
                let mut state = self.state.lock().unwrap();
                state.active -= 1;
                self.terminal = true;
            }
            ExitStatus::from_raw(if success { 0 } else { 256 })
        }
    }

    impl ChildProcess for FakeChild {
        fn try_wait(&mut self) -> io::Result<Option<ExitStatus>> {
            let completed = self
                .state
                .lock()
                .unwrap()
                .completed
                .get(&self.command)
                .copied();
            Ok(completed.map(|success| self.terminal_status(success)))
        }

        fn shutdown(
            &mut self,
            _signal: nix::sys::signal::Signal,
            _grace: Duration,
            _verbose: bool,
        ) -> ShutdownOutcome {
            self.state
                .lock()
                .unwrap()
                .shutdown
                .insert(self.command.clone());
            ShutdownOutcome::Terminated(self.terminal_status(true))
        }
    }

    impl ProcessRunner for FakeRunner {
        fn spawn(
            &self,
            task: &str,
            command: &CommandLine,
            _context: &TaskContext,
            _capture: Option<Arc<CaptureHandle>>,
        ) -> Result<Box<dyn ChildProcess>, String> {
            let command = command.display();
            if command == "spawn-error" {
                return Err("synthetic spawn failure".to_owned());
            }
            let mut state = self.state.lock().unwrap();
            state.active += 1;
            state.max_active = state.max_active.max(state.active);
            state.started.push((task.to_owned(), command.clone()));
            drop(state);
            Ok(Box::new(FakeChild {
                command,
                state: Arc::clone(&self.state),
                terminal: false,
            }))
        }
    }

    fn fake_executor(runner: FakeRunner, limit: usize, fail_fast: bool) -> Executor {
        Executor::new(
            Arc::new(runner),
            Arc::new(FixedClock),
            limit,
            Arc::new(|_| {}),
            fail_fast,
            false,
        )
        .unwrap()
    }

    #[test]
    fn parallel_stage_overlaps_only_up_to_the_configured_bound_without_sleeps() {
        let runner = FakeRunner::default();
        let executor = fake_executor(runner.clone(), 2, false);
        let plan = RunPlan::from_rules(vec![
            task("A", Some("checks"), &["a"]),
            task("B", Some("checks"), &["b"]),
            task("C", Some("checks"), &["c"]),
        ]);
        let mut run = executor.start(RunMetadata::new(1, "test"), plan);

        assert!(matches!(executor.advance(&mut run), Step::Running));
        let started: HashSet<String> = runner.started_commands().into_iter().collect();
        assert_eq!(started, HashSet::from(["a".to_owned(), "b".to_owned()]));
        assert_eq!(runner.state.lock().unwrap().max_active, 2);

        runner.complete("a", true);
        assert!(matches!(executor.advance(&mut run), Step::Running));
        assert!(runner.started_commands().contains(&"c".to_owned()));
        assert_eq!(runner.state.lock().unwrap().active, 2);

        runner.complete("b", true);
        runner.complete("c", true);
        assert!(matches!(executor.advance(&mut run), Step::Finished));
        assert!(executor.finish(run).outcome.is_success());
    }

    #[test]
    fn barrier_and_task_command_order_are_preserved() {
        let runner = FakeRunner::default();
        let executor = fake_executor(runner.clone(), 3, false);
        let plan = RunPlan::from_rules(vec![
            task("A", Some("checks"), &["a1", "a2"]),
            task("B", Some("checks"), &["b"]),
            task("D", None, &["d"]),
        ]);
        let mut run = executor.start(RunMetadata::new(1, "test"), plan);

        executor.advance(&mut run);
        assert!(!runner.started_commands().contains(&"a2".to_owned()));
        assert!(!runner.started_commands().contains(&"d".to_owned()));

        runner.complete("a1", true);
        executor.advance(&mut run);
        assert!(runner.started_commands().contains(&"a2".to_owned()));
        assert!(!runner.started_commands().contains(&"d".to_owned()));

        runner.complete("a2", true);
        executor.advance(&mut run);
        assert!(!runner.started_commands().contains(&"d".to_owned()));

        runner.complete("b", true);
        executor.advance(&mut run);
        assert!(runner.started_commands().contains(&"d".to_owned()));
    }

    #[test]
    fn concurrency_one_matches_sequential_task_order() {
        let runner = FakeRunner::default();
        let executor = fake_executor(runner.clone(), 1, false);
        let plan = RunPlan::from_rules(vec![
            task("A", Some("checks"), &["a"]),
            task("B", Some("checks"), &["b"]),
        ]);
        let mut run = executor.start(RunMetadata::new(1, "test"), plan);

        executor.advance(&mut run);
        assert_eq!(runner.started_commands(), vec!["a"]);
        runner.complete("a", true);
        executor.advance(&mut run);
        assert_eq!(runner.started_commands(), vec!["a", "b"]);
        assert_eq!(runner.state.lock().unwrap().max_active, 1);
    }

    #[test]
    fn sibling_failure_without_fail_fast_preserves_group_and_later_stage() {
        let runner = FakeRunner::default();
        let executor = fake_executor(runner.clone(), 2, false);
        let plan = RunPlan::from_rules(vec![
            task("A", Some("checks"), &["a"]),
            task("B", Some("checks"), &["b"]),
            task("D", None, &["d"]),
        ]);
        let mut run = executor.start(RunMetadata::new(1, "test"), plan);
        executor.advance(&mut run);

        runner.complete("a", false);
        runner.complete("b", true);
        executor.advance(&mut run);
        assert!(runner.started_commands().contains(&"d".to_owned()));
        runner.complete("d", true);
        assert!(matches!(executor.advance(&mut run), Step::Finished));
        let completed = executor.finish(run);
        assert!(completed.outcome.has_failures());
        assert!(!completed.outcome.is_cancelled());
    }

    #[test]
    fn fail_fast_cancels_active_siblings_and_skips_queued_work() {
        let runner = FakeRunner::default();
        let executor = fake_executor(runner.clone(), 2, true);
        let plan = RunPlan::from_rules(vec![
            task("A", Some("checks"), &["a"]),
            task("B", Some("checks"), &["b"]),
            task("C", Some("checks"), &["c"]),
            task("D", None, &["d"]),
        ]);
        let mut run = executor.start(RunMetadata::new(1, "test"), plan);
        executor.advance(&mut run);

        runner.complete("a", false);
        assert!(matches!(executor.advance(&mut run), Step::Finished));
        let completed = executor.finish(run);
        assert!(completed.outcome.has_failures());
        assert!(runner.state.lock().unwrap().shutdown.contains("b"));
        assert!(!runner.started_commands().contains(&"c".to_owned()));
        assert!(!runner.started_commands().contains(&"d".to_owned()));
    }

    #[test]
    fn restart_cancellation_reaps_every_active_child_and_drops_queue() {
        let runner = FakeRunner::default();
        let executor = fake_executor(runner.clone(), 2, false);
        let plan = RunPlan::from_rules(vec![
            task("A", Some("checks"), &["a"]),
            task("B", Some("checks"), &["b"]),
            task("C", Some("checks"), &["c"]),
        ]);
        let mut run = executor.start(RunMetadata::new(1, "test"), plan);
        executor.advance(&mut run);

        executor.cancel(&mut run, None);
        let state = runner.state.lock().unwrap();
        assert_eq!(
            state.shutdown,
            HashSet::from(["a".to_owned(), "b".to_owned()])
        );
        assert_eq!(state.active, 0);
        assert!(!state.started.iter().any(|(_, command)| command == "c"));
    }

    #[test]
    fn spawn_failure_releases_capacity_and_reports_task_failure() {
        let runner = FakeRunner::default();
        let executor = fake_executor(runner.clone(), 1, false);
        let plan = RunPlan::from_rules(vec![
            task("A", Some("checks"), &["spawn-error"]),
            task("B", Some("checks"), &["b"]),
        ]);
        let mut run = executor.start(RunMetadata::new(1, "test"), plan);

        assert!(matches!(executor.advance(&mut run), Step::Running));
        assert_eq!(runner.started_commands(), vec!["b"]);
        runner.complete("b", true);
        assert!(matches!(executor.advance(&mut run), Step::Finished));
        assert!(executor.finish(run).outcome.has_failures());
    }

    #[test]
    fn executor_records_captured_output_per_generation() {
        use crate::output::OutputRegistry;
        let outputs = Arc::new(OutputRegistry::new());
        let executor = Executor::with_outputs(
            Arc::new(SystemProcessRunner),
            Arc::new(SystemClock),
            1,
            Arc::new(|_| {}),
            false,
            false,
            Some(outputs.clone()),
        )
        .expect("concurrency one is supported");

        let plan = RunPlan::from_rules(vec![Rules::new(
            "t".to_owned(),
            vec!["echo hello".to_owned()],
            vec!["src/**".to_owned()],
            vec![],
            false,
        )]);
        let completed = executor.run_to_completion(RunMetadata::new(5, "test"), plan);
        assert!(completed.outcome.is_success());

        let retrieved = outputs.retrieve(5, Some("t"), None, None, false).unwrap();
        assert_eq!(
            retrieved.tasks[0].stdout.as_ref().expect("stdout").content,
            "hello\n"
        );
        assert!(retrieved.tasks[0]
            .stderr
            .as_ref()
            .is_none_or(|stream| stream.content.is_empty()));
    }

    #[test]
    fn started_event_carries_batch_predecessor_and_changed_set() {
        let events = Arc::new(Mutex::new(Vec::new()));
        let recorded = Arc::clone(&events);
        let executor = Executor::new(
            Arc::new(SystemProcessRunner),
            Arc::new(SystemClock),
            1,
            Arc::new(move |event| recorded.lock().unwrap().push(event)),
            false,
            false,
        )
        .expect("concurrency one is supported");

        let metadata = RunMetadata::correlated(
            9,
            "src/a.rs",
            Some(3),
            Some(8),
            vec!["src/a.rs".to_owned(), "src/b.rs".to_owned()],
        );
        let completed = executor.run_to_completion(
            metadata,
            RunPlan::from_rules(vec![task("task", None, &["true"])]),
        );
        assert!(completed.outcome.is_success());

        let events = events.lock().unwrap();
        match events.first() {
            Some(Event::Started {
                run_id: 9,
                batch: Some(3),
                predecessor: Some(8),
                changed,
                ..
            }) => assert_eq!(changed, &vec!["src/a.rs".to_owned(), "src/b.rs".to_owned()]),
            other => panic!("unexpected started event: {:?}", other),
        }
        match events.last() {
            Some(Event::Finished {
                run_id: 9,
                superseded_by: None,
                ..
            }) => {}
            other => panic!("unexpected finished event: {:?}", other),
        }
    }

    #[test]
    fn executor_rejects_zero_and_accepts_bounded_parallelism() {
        let zero = Executor::new(
            Arc::new(SystemProcessRunner),
            Arc::new(SystemClock),
            0,
            Arc::new(|_| {}),
            true,
            false,
        );
        assert!(zero.is_err(), "zero cannot bound execution");

        let parallel = Executor::new(
            Arc::new(SystemProcessRunner),
            Arc::new(SystemClock),
            2,
            Arc::new(|_| {}),
            true,
            false,
        );
        assert!(parallel.is_ok(), "positive limits must be accepted");
    }
}
