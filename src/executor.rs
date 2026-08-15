//! Bounded task execution engine (TASK-0026/TASK-0027).
//!
//! One executor owns process spawn, polling, fail-fast, cancellation, outcome
//! collection, timing, lifecycle events, and stage barriers. Wait and restart
//! policies only decide how plans are submitted or replaced.

use crate::cmd::{self, CaptureHandle, LoggedChild, ShutdownOutcome};
use crate::diagnostics;
use crate::logging;
use crate::output::OutputRegistry;
use crate::plan::{
    ExecutionSignature, RunOutcome, RunPlan, Stage, TaskContext, TaskOutcome, TaskPlan,
};
use crate::rules::CommandLine;
use crate::stdout;

/// Bounded service restart attempts on unexpected exit (TASK-0035).
pub const SERVICE_MAX_RESTARTS: usize = 3;
/// Backoff between service restarts (TASK-0035).
pub const SERVICE_RESTART_BACKOFF_MS: u64 = 500;
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
        /// Exact configured target name for target runs (TASK-0054); None for
        /// filesystem/init/emit runs. Structural — never parsed from trigger.
        target: Option<String>,
        /// Stable execution signature for target runs; the profile identity
        /// that duration history records against (TASK-0054).
        execution_signature: Option<ExecutionSignature>,
        /// Per-generation effective concurrency (TASK-0073): Some(1) for a
        /// sequential override generation; None means the configured bound.
        effective_concurrency: Option<usize>,
        /// Override source label (TASK-0073): "control" for an exact
        /// control-generation override; None for configured/native runs.
        concurrency_source: Option<&'static str>,
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
        label: Option<String>,
        quiet: bool,
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
        label: Option<String>,
        quiet: bool,
    ) -> Result<Box<dyn ChildProcess>, String> {
        let child = match command {
            CommandLine::Shell(command) => {
                cmd::spawn_in_with_capture_quiet(command, context, capture, label, quiet)
            }
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
    /// Exact configured target name for target runs (TASK-0054); None for
    /// filesystem/init/emit runs. Structural — never parsed from trigger.
    pub target: Option<String>,
    /// Stable execution signature for target runs; the profile identity that
    /// duration history records against (TASK-0054).
    pub execution_signature: Option<ExecutionSignature>,
    /// Per-generation effective concurrency (TASK-0073): Some(1) for a
    /// sequential override run; None keeps the executor's configured limit.
    pub effective_concurrency: Option<usize>,
    /// Override source label (TASK-0073): "control" when this generation was
    /// explicitly requested sequential over the control socket.
    pub concurrency_source: Option<&'static str>,
    /// Run-level terminal hooks (TASK-0040): `success`/`failure` commands run
    /// once at the generation terminal outcome.
    pub hooks: crate::config::RunHooks,
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
            target: None,
            execution_signature: None,
            effective_concurrency: None,
            concurrency_source: None,
            hooks: crate::config::RunHooks::default(),
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
            target: None,
            execution_signature: None,
            effective_concurrency: None,
            concurrency_source: None,
            hooks: crate::config::RunHooks::default(),
        }
    }

    /// Attaches the exact target name and its stable execution signature
    /// (TASK-0054). Structural identity; the recorder never parses the
    /// trigger string to recover the target.
    pub fn with_duration_profile(
        mut self,
        target: Option<String>,
        execution_signature: Option<ExecutionSignature>,
    ) -> Self {
        self.target = target;
        self.execution_signature = execution_signature;
        self
    }

    /// Attaches a per-generation effective concurrency override (TASK-0073):
    /// Some(1) for a sequential control run, None for the configured bound.
    pub fn with_effective_concurrency(mut self, effective_concurrency: Option<usize>) -> Self {
        self.effective_concurrency = effective_concurrency;
        self
    }

    /// Attaches the override source label (TASK-0073): "control" when the
    /// generation was explicitly requested sequential over the control socket.
    pub fn with_concurrency_source(mut self, concurrency_source: Option<&'static str>) -> Self {
        self.concurrency_source = concurrency_source;
        self
    }

    /// Attaches run-level terminal hooks (TASK-0040).
    pub fn with_hooks(mut self, hooks: crate::config::RunHooks) -> Self {
        self.hooks = hooks;
        self
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
    /// Position of the next command to spawn within this task (1-based),
    /// and the total command count; drive the `command=1/3` diagnostics.
    command_index: usize,
    command_total: usize,
    /// Per-job output policy (TASK-0041).
    output: crate::config::OutputPolicy,
    /// Managed long-running service (TASK-0035).
    service: bool,
    /// Unexpected-exit restart attempts remaining for a service (TASK-0035).
    service_restarts_left: usize,
}

impl From<TaskPlan> for ActiveTask {
    fn from(task: TaskPlan) -> Self {
        Self {
            name: task.name,
            position: task.position,
            commands: task.commands.clone().into(),
            child: None,
            current_command: None,
            failures: vec![],
            context: task.context,
            context_validated: false,
            group_occurrence: task.group_occurrence,
            started: None,
            capture: None,
            command_index: 0,
            command_total: task.commands.len(),
            output: task.output,
            service: task.service,
            service_restarts_left: crate::executor::SERVICE_MAX_RESTARTS,
        }
    }
}

pub struct Run {
    stages: VecDeque<Stage>,
    queued: VecDeque<TaskPlan>,
    active: Vec<ActiveTask>,
    /// Running managed services (TASK-0035): spawned, alive, and NOT blocking
    /// later stages. Reaped on cancellation/supersession/shutdown.
    services: Vec<ActiveTask>,
    stage_limit: usize,
    results: Vec<Result<(), String>>,
    outcomes: Vec<(usize, String, Option<String>, TaskOutcome)>,
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
            target: metadata.target.clone(),
            execution_signature: metadata.execution_signature.clone(),
            effective_concurrency: metadata.effective_concurrency,
            concurrency_source: metadata.concurrency_source,
        });

        Run {
            stages: plan.stages.into(),
            queued: VecDeque::new(),
            active: vec![],
            services: vec![],
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
                    // TASK-0035: background services keep the generation
                    // alive (polled for restart/failure) until reaped.
                    if !run.services.is_empty() {
                        self.advance_services(run);
                        return Step::Running;
                    }
                    return Step::Finished;
                };
                match stage {
                    Stage::Serial(task) => {
                        run.stage_limit = 1;
                        run.queued.push_back(task);
                    }
                    Stage::Parallel { tasks, .. } => {
                        // TASK-0073: a per-generation sequential override
                        // caps this stage at one; otherwise the configured
                        // executor limit applies.
                        let limit = run
                            .metadata
                            .effective_concurrency
                            .unwrap_or(self.concurrency_limit);
                        run.stage_limit = limit.min(tasks.len());
                        run.queued.extend(tasks);
                    }
                }
            }

            self.fill_available_slots(run);
            let mut task_finished = false;
            let mut index = 0;
            while index < run.active.len() {
                match self.advance_task(
                    &mut run.active[index],
                    &mut run.results,
                    run.metadata.run_id,
                ) {
                    TaskStep::Running => {
                        // TASK-0035: a spawned-and-running service is moved
                        // to the background set so it never blocks later
                        // stages; it is reaped at cancellation/shutdown.
                        if run.active[index].service {
                            let service = run.active.remove(index);
                            run.services.push(service);
                            continue;
                        }
                        index += 1;
                    }
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

            // TASK-0035: background services are polled for unexpected exit
            // (restart with bound) without blocking stage progression.
            if !run.services.is_empty() {
                self.advance_services(run);
            }

            if run.active.is_empty() && run.queued.is_empty() {
                // TASK-0035: background services keep the generation alive
                // until superseded/cancelled/finished, so their restart and
                // failure policy is polled; a generation with only services
                // is still Running (scheduled) rather than Finished.
                if run.services.is_empty() {
                    continue;
                }
                return Step::Running;
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
        run_id: u64,
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
                // Capture whenever a retention registry exists OR the task
                // needs buffered output for its policy (TASK-0041):
                // quiet/capture/show-on-failure hold output for retrieval or
                // reveal even when no control surface is wired.
                if task.capture.is_none()
                    && (self.outputs.is_some()
                        || task.output != crate::config::OutputPolicy::Inherit)
                {
                    task.capture = Some(Arc::new(CaptureHandle::new()));
                }
                match self.runner.spawn(
                    &task.name,
                    &command,
                    &task.context,
                    task.capture.clone(),
                    // TASK-0028: attribute live lines to the task only when
                    // it runs in a parallel group, where output can interleave.
                    // Serial tasks keep today's raw passthrough (contract §7).
                    task.group_occurrence.is_some().then(|| task.name.clone()),
                    // TASK-0041: quiet/capture/show-on-failure suppress live
                    // output; inherit streams it.
                    !matches!(task.output, crate::config::OutputPolicy::Inherit),
                ) {
                    Ok(child) => {
                        task.child = Some(child);
                        task.command_index += 1;
                        if task.started.is_none() {
                            task.started = Some(self.clock.now());
                        }
                        if self.verbose {
                            diagnostics::debug(&diagnostics::Record {
                                generation: Some(run_id),
                                command_position: Some((task.command_index, task.command_total)),
                                state: Some("started"),
                                command: Some(display.clone()),
                                ..Default::default()
                            });
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
                    let service_command = task.current_command.clone();
                    task.current_command = None;
                    // TASK-0035: a managed service restarts on unexpected
                    // (non-zero) exit with bounded attempts and backoff; a
                    // zero exit is a deliberate stop. A running service never
                    // finishes the generation — it returns Running until
                    // superseded or shut down.
                    if task.service {
                        if status.success() {
                            // Deliberate stop: the service is done for this
                            // generation (e.g. it exited on its own request).
                            results.push(Ok(()));
                            return TaskStep::Finished;
                        }
                        if task.service_restarts_left > 0 {
                            task.service_restarts_left -= 1;
                            stdout::warn(&format!(
                                "service '{}' exited with {}; restarting ({} left)",
                                task.name, status, task.service_restarts_left
                            ));
                            self.clock
                                .sleep(Duration::from_millis(SERVICE_RESTART_BACKOFF_MS));
                            // The service command was consumed at spawn; put it
                            // back so the next loop iteration respawns it.
                            if let Some(cmd) = &service_command {
                                task.commands.push_front(CommandLine::Shell(cmd.clone()));
                            }
                            continue;
                        }
                        let failure = format!(
                            "Service {} has failed after {} restarts",
                            task.name, SERVICE_MAX_RESTARTS
                        );
                        task.failures.push(failure.clone());
                        results.push(Err(failure));
                        return TaskStep::Finished;
                    }

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

    /// Reveals a task's captured output once on failure for the
    /// show-on-failure policy (TASK-0041): streams the buffered stdout/stderr
    /// with task attribution exactly once, so failures are diagnosable while
    /// passing jobs stay quiet.
    fn reveal_on_failure(&self, task: &ActiveTask, failures: &[String]) {
        if task.output != crate::config::OutputPolicy::ShowOnFailure || failures.is_empty() {
            return;
        }
        if let Some(capture) = &task.capture {
            let data = capture.finish();
            let label = task.group_occurrence.as_deref().unwrap_or(&task.name);
            for (bytes, stream) in [
                (data.stdout.bytes(), "stdout"),
                (data.stderr.bytes(), "stderr"),
            ] {
                if bytes.is_empty() {
                    continue;
                }
                let text = String::from_utf8_lossy(bytes);
                for line in text.lines() {
                    let attributed = format!("[{}:{}] {}", label, stream, line);
                    println!("{}", attributed);
                    logging::log_plain(&attributed);
                }
            }
        }
    }

    fn record_task_outcome(&self, run: &mut Run, task: ActiveTask) {
        let task_failed = !task.failures.is_empty();
        self.reveal_on_failure(&task, &task.failures);
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
        run.outcomes.push((
            task.position,
            task.name,
            task.group_occurrence.clone(),
            outcome,
        ));
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
            run.outcomes.push((
                task.position,
                task.name,
                task.group_occurrence.clone(),
                TaskOutcome::Cancelled,
            ));
        }
        for task in run.queued.drain(..) {
            self.record_task_snapshot(
                run.metadata.run_id,
                &task.name,
                task.group_occurrence.as_deref(),
                TaskState::Cancelled,
                None,
            );
            run.outcomes.push((
                task.position,
                task.name,
                task.group_occurrence.clone(),
                TaskOutcome::Skipped,
            ));
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
                run.outcomes.push((
                    task.position,
                    task.name,
                    task.group_occurrence.clone(),
                    TaskOutcome::Skipped,
                ));
            }
        }
    }

    /// Polls background services (TASK-0035): a service that exited
    /// unexpectedly restarts with a bounded attempt count; exceeding the
    /// bound records a failure. Deliberate zero-exit stops remove the
    /// service from the background set.
    fn advance_services(&self, run: &mut Run) {
        let mut index = 0;
        while index < run.services.len() {
            let service = &mut run.services[index];
            // Respawn a restarted service whose child was reaped and whose
            // command was queued for the next attempt.
            if service.child.is_none() {
                let Some(command) = service.commands.pop_front() else {
                    index += 1;
                    continue;
                };
                let display = command.display();
                service.current_command = Some(display.clone());
                match self.runner.spawn(
                    &service.name,
                    &command,
                    &service.context,
                    service.capture.clone(),
                    service
                        .group_occurrence
                        .is_some()
                        .then(|| service.name.clone()),
                    !matches!(service.output, crate::config::OutputPolicy::Inherit),
                ) {
                    Ok(child) => {
                        service.child = Some(child);
                        if service.started.is_none() {
                            service.started = Some(self.clock.now());
                        }
                        if self.verbose {
                            diagnostics::debug(&diagnostics::Record {
                                generation: Some(run.metadata.run_id),
                                command_position: Some((1, 1)),
                                state: Some("started"),
                                command: Some(display.clone()),
                                ..Default::default()
                            });
                        }
                    }
                    Err(err) => {
                        let failure = format!("Service {} respawn failed: {}", service.name, err);
                        service.failures.push(failure.clone());
                        run.results.push(Err(failure));
                        let done = run.services.remove(index);
                        run.outcomes.push((
                            done.position,
                            done.name.clone(),
                            done.group_occurrence.clone(),
                            TaskOutcome::Failed {
                                failures: done.failures.clone(),
                            },
                        ));
                        continue;
                    }
                }
                index += 1;
                continue;
            }
            let child = service.child.as_mut().expect("child present");
            match child.try_wait() {
                Ok(None) => index += 1,
                Ok(Some(status)) => {
                    service.child = None;
                    let command = service.current_command.clone();
                    service.current_command = None;
                    if status.success() {
                        // Deliberate stop: remove from background.
                        let done = run.services.remove(index);
                        run.outcomes.push((
                            done.position,
                            done.name.clone(),
                            done.group_occurrence.clone(),
                            TaskOutcome::Passed,
                        ));
                        continue;
                    }
                    if service.service_restarts_left > 0 {
                        service.service_restarts_left -= 1;
                        stdout::warn(&format!(
                            "service '{}' exited with {}; restarting ({} left)",
                            service.name, status, service.service_restarts_left
                        ));
                        if let Some(cmd) = command {
                            service.commands.push_front(CommandLine::Shell(cmd));
                        }
                        index += 1;
                        continue;
                    }
                    let failure = format!(
                        "Service {} has failed after {} restarts",
                        service.name, SERVICE_MAX_RESTARTS
                    );
                    service.failures.push(failure.clone());
                    run.results.push(Err(failure));
                    let done = run.services.remove(index);
                    run.outcomes.push((
                        done.position,
                        done.name.clone(),
                        done.group_occurrence.clone(),
                        TaskOutcome::Failed {
                            failures: done.failures.clone(),
                        },
                    ));
                    continue;
                }
                Err(_) => index += 1,
            }
        }
    }

    /// Runs the applicable terminal hook once (TASK-0040): success hook on
    /// pass, failure hook on fail. Hook failure never changes the run
    /// outcome; it is surfaced via a warning for loop diagnosis. The hook is
    /// spawned with the same process runner and reaped to completion.
    fn run_terminal_hook(&self, metadata: &RunMetadata, outcome: &RunOutcome) {
        let command = if outcome.is_success() {
            metadata.hooks.success.as_deref()
        } else {
            metadata.hooks.failure.as_deref()
        };
        let Some(command) = command else { return };
        let label = if outcome.is_success() {
            "success"
        } else {
            "failure"
        };
        if self.verbose {
            diagnostics::debug(&diagnostics::Record {
                generation: Some(metadata.run_id),
                source: Some("hook"),
                decision: Some("started"),
                note: Some(format!("{label} hook: {command}")),
                ..Default::default()
            });
        }
        let result = self
            .runner
            .spawn(
                "hook",
                &CommandLine::Shell(command.to_owned()),
                &TaskContext::default(),
                None,
                None,
                false,
            )
            .and_then(|mut child| {
                // Wait via the trait's try_wait (the runner returns a
                // trait object; LoggedChild::try_wait joins forwarding
                // threads on exit, so no output is lost).
                loop {
                    match child.try_wait() {
                        Ok(Some(status)) => return Ok(status),
                        Ok(None) => std::thread::sleep(Duration::from_millis(20)),
                        Err(err) => return Err(format!("hook wait failed: {}", err)),
                    }
                }
            });
        match result {
            Ok(status) if status.success() => {}
            Ok(status) => stdout::warn(&format!(
                "{} hook for generation {} failed with {}",
                label, metadata.run_id, status
            )),
            Err(err) => stdout::warn(&format!(
                "{} hook for generation {} errored: {}",
                label, metadata.run_id, err
            )),
        }
    }

    pub fn finish(&self, mut run: Run) -> CompletedRun {
        let elapsed = self.clock.elapsed(run.started);
        run.outcomes.sort_by_key(|(position, _, _, _)| *position);
        let outcome = RunOutcome::from_task_outcomes(
            run.outcomes
                .into_iter()
                .map(|(_, name, group, outcome)| (name, group, outcome))
                .collect(),
        );
        let failures = outcome.failures();
        if self.verbose {
            diagnostics::debug(&diagnostics::Record {
                generation: Some(run.metadata.run_id),
                state: Some(if outcome.is_success() {
                    "passed"
                } else {
                    "failed"
                }),
                duration: Some(format!("{:.3}s", elapsed.as_secs_f64())),
                ..Default::default()
            });
        }
        // TASK-0040: run the applicable terminal hook (success/failure) once,
        // after the generation's tasks reached terminal. Hook failure never
        // changes the combined outcome or exit code; it is surfaced for
        // diagnosis only. Superseded/cancelled generations never reach here.
        self.run_terminal_hook(&run.metadata, &outcome);
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
        if self.verbose {
            diagnostics::debug(&diagnostics::Record {
                generation: Some(run.metadata.run_id),
                state: Some("cancelled"),
                reason: Some(if superseded_by.is_some() {
                    "replaced".to_owned()
                } else {
                    "requested".to_owned()
                }),
                ..Default::default()
            });
        }
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
        let outcome = child.shutdown(signal, grace, self.verbose);
        let escalated = matches!(outcome, ShutdownOutcome::Escalated { .. });
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
                output: crate::config::OutputPolicy::Inherit,
                service: false,
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
            label: Option<String>,
            quiet: bool,
        ) -> Result<Box<dyn ChildProcess>, String> {
            self.commands.lock().unwrap().push(format!(
                "{}:{}:{}",
                task,
                command.display(),
                label.is_some()
            ));
            SystemProcessRunner.spawn(task, command, context, capture, label, quiet)
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
        assert_eq!(*runner.commands.lock().unwrap(), vec!["task:true:false"]);
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

    #[test]
    fn parallel_group_tasks_get_live_output_labels_serial_tasks_do_not() {
        // TASK-0028: live lines from parallel-group tasks are attributed with
        // the task name so interleaved output keeps identity; serial tasks
        // keep today's raw passthrough.
        let runner = Arc::new(RecordingRunner::default());
        let executor = Executor::new(
            runner.clone(),
            Arc::new(FixedClock),
            2,
            Arc::new(|_| {}),
            false,
            false,
        )
        .expect("bounded concurrency");

        let completed = executor.run_to_completion(
            RunMetadata::new(1, "test"),
            RunPlan::from_rules(vec![
                task("serial", None, &["true"]),
                task("grouped-a", Some("checks"), &["true"]),
                task("grouped-b", Some("checks"), &["true"]),
            ]),
        );
        assert!(completed.outcome.is_success());

        let commands = runner.commands.lock().unwrap();
        // serial task spawns without a label; both group members spawn labeled.
        assert_eq!(commands[0], "serial:true:false");
        assert!(commands.iter().any(|entry| entry == "grouped-a:true:true"));
        assert!(commands.iter().any(|entry| entry == "grouped-b:true:true"));
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
            _label: Option<String>,
            _quiet: bool,
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

    fn service_rule(name: &str, commands: &[&str], service: bool) -> Rules {
        let mut rule = Rules::new(
            name.to_owned(),
            commands.iter().map(|c| c.to_string()).collect(),
            vec!["src/**".to_owned()],
            vec![],
            false,
        );
        if service {
            rule = rule.with_service(true);
        }
        rule
    }

    #[test]
    fn finite_task_exits_are_failures_not_restarts() {
        let runner = FakeRunner::default();
        let executor = fake_executor(runner.clone(), 1, false);
        let plan = RunPlan::from_rules(vec![service_rule("finite", &["boom"], false)]);
        let mut run = executor.start(RunMetadata::new(1, "test"), plan);
        executor.advance(&mut run);
        runner.complete("boom", false);
        assert!(matches!(executor.advance(&mut run), Step::Finished));
        let completed = executor.finish(run);
        assert!(completed.outcome.has_failures());
        assert_eq!(runner.state.lock().unwrap().started.len(), 1, "no restarts");
    }

    #[test]
    fn service_restarts_on_unexpected_exit_up_to_the_bound() {
        let runner = FakeRunner::default();
        let executor = fake_executor(runner.clone(), 1, false);
        let plan = RunPlan::from_rules(vec![service_rule("svc", &["serve"], true)]);
        let mut run = executor.start(RunMetadata::new(1, "test"), plan);
        executor.advance(&mut run);
        assert_eq!(runner.state.lock().unwrap().started.len(), 1, "first spawn");
        // The fake child is terminal once marked; each advance polls the
        // service and exhausts one restart. Drive past the bound.
        runner.complete("serve", false);
        // Each restart needs a respawn poll + an exit poll; drive well past
        // the bound so the final failure is recorded deterministically.
        for _ in 0..=(4 + SERVICE_MAX_RESTARTS) {
            let _ = executor.advance(&mut run);
        }
        // After the bound, the service failure is recorded in outcomes.
        assert!(
            run.outcomes
                .iter()
                .any(|(_, _, _, o)| matches!(o, TaskOutcome::Failed { .. })),
            "service must fail after the restart bound: {:?}",
            run.outcomes
        );
        assert_eq!(
            runner.state.lock().unwrap().started.len(),
            1 + SERVICE_MAX_RESTARTS,
            "1 + 3 spawns"
        );
    }

    #[test]
    fn running_service_is_background_and_does_not_block_generation() {
        let runner = FakeRunner::default();
        let executor = fake_executor(runner.clone(), 1, false);
        let plan = RunPlan::from_rules(vec![
            service_rule("svc", &["serve"], true),
            service_rule("after", &["next"], false),
        ]);
        let mut run = executor.start(RunMetadata::new(1, "test"), plan);
        // The service spawns and is moved to the background set; the next
        // stage (finite work) proceeds without waiting for the service.
        executor.advance(&mut run);
        runner.complete("next", true);
        executor.advance(&mut run);
        assert_eq!(
            runner.state.lock().unwrap().started.len(),
            2,
            "both spawned"
        );
        let completed = executor.finish(run);
        assert!(
            completed.outcome.is_success(),
            "service never blocks later work"
        );
    }
}
