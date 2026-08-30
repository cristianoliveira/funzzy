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
use serde::Serialize;
use std::collections::VecDeque;
use std::io;
use std::process::ExitStatus;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;
use std::time::{Duration, Instant};

const POLL_INTERVAL: Duration = Duration::from_millis(10);

/// Cancellation signal shared by the scheduler, executor, and approval
/// adapter. It lets a replacement or exact cancel interrupt a blocking
/// recovery boundary before the worker can process the queued command.
#[derive(Clone, Default)]
pub struct CancellationToken(Arc<AtomicBool>);

impl CancellationToken {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn cancel(&self) {
        self.0.store(true, Ordering::SeqCst);
    }

    pub fn is_cancelled(&self) -> bool {
        self.0.load(Ordering::SeqCst)
    }
}

/// Wire-level task state for the correlated snapshot (contract §7). `Skipped`
/// (fail-fast skipped work) collapses to `Cancelled` — never-started work is
/// reported as cancelled, matching the pi-watcher decoder vocabulary.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize)]
#[serde(rename_all = "lowercase")]
pub enum TaskState {
    Passed,
    Failed,
    Cancelled,
    /// FINITE-JOB-TIMEOUT-CONTRACT §4: additive wire value `timedout` —
    /// distinct from command failure and from client-await timeout.
    TimedOut,
}

/// One task's terminal outcome for the correlated snapshot (TASK-0050).
#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct TaskSnapshot {
    /// Configured declaration position. It orders in-process report projections
    /// but is intentionally absent from the additive control/event wire shape.
    #[serde(skip)]
    pub position: usize,
    pub id: String,
    pub name: String,
    pub state: TaskState,
    pub duration_ms: Option<u64>,
}

/// One exact generation/job recovery approval request. Command text is
/// rendered for the attached user but never used as an authorization key.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct RecoveryRequest {
    pub generation: u64,
    pub revision: Option<u64>,
    pub job_position: usize,
    pub job: String,
    pub commands: Vec<String>,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ApprovalDecision {
    Approved,
    Declined,
    NoTty,
    Eof,
    Invalid,
    TimedOut,
    Cancelled,
}

pub trait RecoveryApproval: Send + Sync {
    fn approve(
        &self,
        requests: &[RecoveryRequest],
        cancellation: &CancellationToken,
        timeout: Duration,
    ) -> ApprovalDecision;
}

/// Safe default used by headless composition until a TTY adapter is injected.
pub struct DenyRecoveryApproval;

impl RecoveryApproval for DenyRecoveryApproval {
    fn approve(
        &self,
        _requests: &[RecoveryRequest],
        cancellation: &CancellationToken,
        _timeout: Duration,
    ) -> ApprovalDecision {
        if cancellation.is_cancelled() {
            ApprovalDecision::Cancelled
        } else {
            ApprovalDecision::Declined
        }
    }
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
        /// Immutable config revision this generation was frozen under
        /// (TASK-0089).
        revision: Option<u64>,
        /// Non-secret semantic hash of the frozen config revision.
        revision_hash: Option<String>,
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
    /// Non-terminal recovery lifecycle evidence for one generation/job.
    RecoveryPhase {
        run_id: u64,
        job: String,
        phase: String,
        outcome: Option<String>,
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
    pub hooks: crate::config::GenerationHooks,
    /// Frozen recovery policy for this generation.
    pub recovery_policy: crate::config::RecoveryPolicy,
    /// Approval-only timeout frozen for this generation.
    pub recovery_timeout: Duration,
    /// Immutable config revision this generation was frozen under
    /// (TASK-0089, CONFIG-RELOAD-CONTRACT §4). None for legacy runs that
    /// never observe reload.
    pub revision: Option<u64>,
    /// Non-secret semantic hash of the frozen config revision (TASK-0089).
    pub revision_hash: Option<String>,
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
            hooks: crate::config::GenerationHooks::default(),
            recovery_policy: crate::config::RecoveryPolicy::Prompt,
            recovery_timeout: Duration::from_secs(60),
            revision: None,
            revision_hash: None,
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
            hooks: crate::config::GenerationHooks::default(),
            recovery_policy: crate::config::RecoveryPolicy::Prompt,
            recovery_timeout: Duration::from_secs(60),
            revision: None,
            revision_hash: None,
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

    /// Attaches the immutable config revision this generation is frozen
    /// under (TASK-0089).
    pub fn with_revision(mut self, revision: u64, revision_hash: String) -> Self {
        self.revision = Some(revision);
        self.revision_hash = Some(revision_hash);
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
    pub fn with_hooks(mut self, hooks: crate::config::GenerationHooks) -> Self {
        self.hooks = hooks;
        self
    }

    /// Attaches the frozen recovery policy for this generation.
    pub fn with_recovery_policy(mut self, policy: crate::config::RecoveryPolicy) -> Self {
        self.recovery_policy = policy;
        self
    }

    pub fn with_recovery_timeout(mut self, timeout: Duration) -> Self {
        self.recovery_timeout = timeout;
        self
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PendingSettledHook {
    pub run_id: u64,
    pub command: String,
    pub settle: Duration,
    pub revision: u64,
    pub revision_hash: String,
}

pub struct CompletedRun {
    pub results: Vec<Result<(), String>>,
    pub elapsed: Duration,
    pub outcome: RunOutcome,
    /// Settled failure hook to be coordinated by the worker after publication.
    pub pending_settled_hook: Option<PendingSettledHook>,
    /// Terminal job snapshots in configured declaration order. These carry
    /// the executor's only per-job monotonic duration measurements.
    pub tasks: Vec<TaskSnapshot>,
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
    original_commands: Vec<CommandLine>,
    recovery_commands: Option<VecDeque<CommandLine>>,
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
    output: crate::rules::OutputPolicy,
    /// Managed long-running service (TASK-0035).
    service: bool,
    /// Unexpected-exit restart attempts remaining for a service (TASK-0035).
    service_restarts_left: usize,
    /// Defer original command errors while recovery may change the outcome.
    defer_failure: bool,
    /// FINITE-JOB-TIMEOUT-CONTRACT §4: terminal marker for the typed
    /// TimedOut task state (distinct from command failure).
    timed_out: bool,
    /// Frozen finite deadline from the task plan (FINITE-JOB-TIMEOUT-
    /// CONTRACT §7); computed against `started` once, never reset by
    /// continuation spawns (§3 job-wide rule).
    deadline: Option<Instant>,
    timeout: Option<Duration>,
}

impl From<TaskPlan> for ActiveTask {
    fn from(task: TaskPlan) -> Self {
        Self {
            name: task.name,
            position: task.position,
            commands: task.commands.clone().into(),
            original_commands: task.commands.clone(),
            recovery_commands: task.recovery_commands.clone().map(VecDeque::from),
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
            defer_failure: task.recovery_commands.is_some(),
            timed_out: false,
            deadline: None,
            timeout: task.timeout,
        }
    }
}

pub struct Run {
    stages: VecDeque<Stage>,
    queued: VecDeque<TaskPlan>,
    active: Vec<ActiveTask>,
    /// Original failures eligible for one post-stage recovery pass.
    pending_recoveries: Vec<ActiveTask>,
    /// Running managed services (TASK-0035): spawned, alive, and NOT blocking
    /// later stages. Reaped on cancellation/supersession/shutdown.
    services: Vec<ActiveTask>,
    stage_limit: usize,
    results: Vec<Result<(), String>>,
    outcomes: Vec<(usize, String, Option<String>, TaskOutcome)>,
    /// Terminal snapshots retain their configured position so every projection
    /// is deterministic when parallel jobs complete out of order.
    task_snapshots: Vec<TaskSnapshot>,
    metadata: RunMetadata,
    superseded_by: Option<u64>,
    cancellation: CancellationToken,
    started: Instant,
}

impl Run {
    /// The generation identity of this run.
    pub fn run_id(&self) -> u64 {
        self.metadata.run_id
    }

    /// The frozen config revision this run was scheduled under (TASK-0091,
    /// AC2): reported by `cancel` so the cancellation is attributable to the
    /// exact revision; None for legacy runs that never observe reload.
    pub fn revision(&self) -> Option<crate::config_revision::ConfigRevision> {
        match (
            self.metadata.revision,
            self.metadata.revision_hash.as_deref(),
        ) {
            (Some(number), Some(hash)) => Some(crate::config_revision::ConfigRevision {
                number,
                hash: hash.to_owned(),
            }),
            _ => None,
        }
    }

    pub(crate) fn cancellation_token(&self) -> CancellationToken {
        self.cancellation.clone()
    }

    fn cancellation_requested(&self) -> bool {
        self.cancellation.is_cancelled()
    }
}

enum TaskStep {
    Running,
    Finished,
    FailedFast,
    /// FINITE-JOB-TIMEOUT-CONTRACT §3/§4: the job-wide deadline elapsed
    /// before the child was reaped; the process group was terminated.
    TimedOut,
}

pub struct Executor {
    runner: Arc<dyn ProcessRunner>,
    clock: Arc<dyn Clock>,
    /// Task concurrency bound (TASK-0054). Interior-mutable (TASK-0090): a
    /// config reload swaps the bound so newly planned generations use the
    /// committed revision's concurrency while a running group keeps the limit
    /// it was planned under (AC7 — never resize a running group).
    concurrency_limit: Arc<std::sync::atomic::AtomicUsize>,
    events: Arc<dyn EventSink>,
    fail_fast: bool,
    verbose: bool,
    /// Retained-output registry fed at task terminal (TASK-0045); None keeps
    /// capture disabled (no control surface consumes it).
    outputs: Option<Arc<OutputRegistry>>,
    /// Injected approval boundary; domain code never reads global stdin.
    approval: Arc<dyn RecoveryApproval>,
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
            concurrency_limit: Arc::new(std::sync::atomic::AtomicUsize::new(concurrency_limit)),
            events,
            fail_fast,
            verbose,
            outputs: None,
            approval: Arc::new(DenyRecoveryApproval),
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
            concurrency_limit: Arc::new(std::sync::atomic::AtomicUsize::new(concurrency_limit)),
            events,
            fail_fast,
            verbose,
            outputs,
            approval: Arc::new(DenyRecoveryApproval),
        })
    }

    /// Injects the approval adapter used by future generations.
    pub fn with_recovery_approval(mut self, approval: Arc<dyn RecoveryApproval>) -> Self {
        self.approval = approval;
        self
    }

    pub fn concurrency_limit(&self) -> usize {
        self.concurrency_limit
            .load(std::sync::atomic::Ordering::SeqCst)
    }

    /// Swaps the task concurrency bound (TASK-0090 AC7): stages planned AFTER
    /// this call use the new bound; a running group keeps the `stage_limit`
    /// it was planned under and is never resized inconsistently.
    pub fn set_concurrency_limit(&self, limit: usize) {
        assert!(limit > 0, "executor concurrency limit must be positive");
        self.concurrency_limit
            .store(limit, std::sync::atomic::Ordering::SeqCst);
    }

    /// The shared bound handle; lets the worker swap the same value the
    /// executor reads (TASK-0090 AC7).
    pub fn concurrency_handle(&self) -> Arc<std::sync::atomic::AtomicUsize> {
        Arc::clone(&self.concurrency_limit)
    }

    /// Adopts an externally owned bound handle so the worker and executor
    /// share one value (TASK-0090 AC7): a reload swap through the worker
    /// immediately affects stages the executor plans afterwards.
    pub fn with_concurrency_handle(mut self, handle: Arc<std::sync::atomic::AtomicUsize>) -> Self {
        assert!(
            handle.load(std::sync::atomic::Ordering::SeqCst) > 0,
            "executor concurrency limit must be positive"
        );
        self.concurrency_limit = handle;
        self
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
            revision: metadata.revision,
            revision_hash: metadata.revision_hash.clone(),
        });

        Run {
            stages: plan.stages.into(),
            queued: VecDeque::new(),
            active: vec![],
            pending_recoveries: vec![],
            services: vec![],
            stage_limit: 0,
            results: vec![],
            outcomes: vec![],
            task_snapshots: vec![],
            metadata,
            superseded_by: None,
            cancellation: CancellationToken::new(),
            started: self.clock.now(),
        }
    }

    pub fn advance(&self, run: &mut Run) -> Step {
        if run.cancellation_requested() {
            return Step::Running;
        }

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
                            .unwrap_or_else(|| self.concurrency_limit());
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
                    self.fail_fast,
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
                        self.defer_or_record(run, task);
                        task_finished = true;
                    }
                    TaskStep::FailedFast => {
                        let task = run.active.remove(index);
                        self.defer_or_record(run, task);
                        self.stop_after_failure(run);
                        if !run.pending_recoveries.is_empty() && self.resolve_recoveries(run) {
                            return Step::Running;
                        }
                        return Step::Finished;
                    }
                    TaskStep::TimedOut => {
                        // FINITE-JOB-TIMEOUT-CONTRACT §3/§7: a timeout is a
                        // failure — record the typed outcome and apply
                        // fail-fast to siblings exactly as for command
                        // failures: with fail_fast the generation stops
                        // (active siblings cancelled, queued work skipped);
                        // without it the generation continues and later work
                        // still runs. Recovery is never offered (§7).
                        let task = run.active.remove(index);
                        self.record_task_outcome(run, task);
                        if self.fail_fast {
                            self.stop_after_failure(run);
                            return Step::Finished;
                        }
                        task_finished = true;
                    }
                }
            }

            // TASK-0035: background services are polled for unexpected exit
            // (restart with bound) without blocking stage progression.
            if !run.services.is_empty() {
                self.advance_services(run);
            }

            if run.active.is_empty() && run.queued.is_empty() {
                if !run.pending_recoveries.is_empty() {
                    if self.resolve_recoveries(run) {
                        return Step::Running;
                    }
                    continue;
                }
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
        fail_fast: bool,
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
                    if !task.defer_failure {
                        results.push(Err(failure));
                    }
                    task.commands.clear();
                    return if fail_fast {
                        TaskStep::FailedFast
                    } else {
                        TaskStep::Finished
                    };
                }
            }
        }

        loop {
            if task.child.is_none() {
                // FINITE-JOB-TIMEOUT-CONTRACT §3 sequential recheck: the
                // ORIGINAL deadline governs the job's whole invocation —
                // recheck it BEFORE any continuation spawn so an expired
                // budget never starts a command it must immediately kill.
                if let Some(deadline) = task.deadline {
                    if self.clock.now() >= deadline {
                        return self.expire_task(task, results);
                    }
                }
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
                        || task.output != crate::rules::OutputPolicy::Inherit)
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
                    !matches!(task.output, crate::rules::OutputPolicy::Inherit),
                ) {
                    Ok(child) => {
                        task.child = Some(child);
                        task.command_index += 1;
                        if task.started.is_none() {
                            task.started = Some(self.clock.now());
                            // FINITE-JOB-TIMEOUT-CONTRACT §2/§3: the deadline
                            // is minted ONCE from the job's first successful
                            // spawn; continuation spawns reuse it (never
                            // reset, never fresh).
                            if let (Some(timeout), None) = (task.timeout, task.deadline) {
                                task.deadline = task.started.map(|started| started + timeout);
                            }
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
                        if !task.defer_failure {
                            results.push(Err(failure));
                        }
                        task.current_command = None;
                        if fail_fast {
                            return TaskStep::FailedFast;
                        }
                    }
                }
                continue;
            }

            let display = task.current_command.clone().unwrap_or_default();
            // FINITE-JOB-TIMEOUT-CONTRACT §3: single ordering rule — the
            // timeout check precedes try_wait in every iteration, so a
            // child that exited at deadline−ε but is reaped in a later poll
            // is a timeout outcome (indeterminism bounded by one interval).
            if let Some(deadline) = task.deadline {
                if self.clock.now() >= deadline {
                    return self.expire_task(task, results);
                }
            }
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
                    if !task.defer_failure {
                        results.push(Err(failure));
                    }
                    if fail_fast {
                        return TaskStep::FailedFast;
                    }
                }
                Err(err) => {
                    task.child = None;
                    task.current_command = None;
                    let failure = format!("Command {} has errored with {}", display, err);
                    task.failures.push(failure.clone());
                    if !task.defer_failure {
                        results.push(Err(failure));
                    }
                    if fail_fast {
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
        run: &mut Run,
        position: usize,
        name: &str,
        group_occurrence: Option<&str>,
        state: TaskState,
        duration_ms: Option<u64>,
    ) {
        let task = TaskSnapshot {
            position,
            id: group_occurrence
                .map(str::to_owned)
                .unwrap_or_else(|| name.to_owned()),
            name: name.to_owned(),
            state,
            duration_ms,
        };
        run.task_snapshots.push(task.clone());
        self.events.emit(Event::TaskTerminal {
            run_id: run.metadata.run_id,
            task,
        });
    }

    /// Reveals a task's captured output once on failure for the
    /// show-on-failure policy (TASK-0041): streams the buffered stdout/stderr
    /// with task attribution exactly once, so failures are diagnosable while
    /// passing jobs stay quiet.
    fn reveal_on_failure(&self, task: &ActiveTask, failures: &[String]) {
        if task.output != crate::rules::OutputPolicy::ShowOnFailure || failures.is_empty() {
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

    fn defer_or_record(&self, run: &mut Run, task: ActiveTask) {
        if !task.failures.is_empty() && task.recovery_commands.is_some() {
            self.events.emit(Event::RecoveryPhase {
                run_id: run.metadata.run_id,
                job: task.name.clone(),
                phase: "original_failed".to_owned(),
                outcome: None,
            });
            run.pending_recoveries.push(task);
        } else {
            self.record_task_outcome(run, task);
        }
    }

    /// Resolves one bounded recovery pass. Returns `true` when cancellation
    /// interrupted approval, recovery, or verification; the worker then
    /// consumes the queued cancel and reaps the preserved task state.
    fn resolve_recoveries(&self, run: &mut Run) -> bool {
        run.pending_recoveries.sort_by_key(|task| task.position);
        let mut pending = std::mem::take(&mut run.pending_recoveries);
        if pending.is_empty() {
            return false;
        }
        if run.cancellation_requested() {
            run.pending_recoveries.append(&mut pending);
            return true;
        }

        let requests: Vec<RecoveryRequest> = pending
            .iter()
            .map(|task| RecoveryRequest {
                generation: run.metadata.run_id,
                revision: run.metadata.revision,
                job_position: task.position,
                job: task.name.clone(),
                commands: task
                    .recovery_commands
                    .as_ref()
                    .into_iter()
                    .flat_map(|commands| commands.iter())
                    .map(CommandLine::display)
                    .collect(),
            })
            .collect();
        for request in &requests {
            self.events.emit(Event::RecoveryPhase {
                run_id: request.generation,
                job: request.job.clone(),
                phase: "approval_requested".to_owned(),
                outcome: None,
            });
        }

        let decision = if matches!(
            run.metadata.recovery_policy,
            crate::config::RecoveryPolicy::Prompt
        ) {
            let approval = Arc::clone(&self.approval);
            let requested = requests.clone();
            let timeout = run.metadata.recovery_timeout;
            let approval_cancellation = CancellationToken::new();
            let approval_signal = approval_cancellation.clone();
            let (sender, receiver) = std::sync::mpsc::channel();
            std::thread::spawn(move || {
                let decision = approval.approve(&requested, &approval_signal, timeout);
                let _ = sender.send(decision);
            });
            let deadline = Instant::now() + timeout;
            loop {
                if run.cancellation_requested() {
                    // The approval adapter owns the TTY read. Wait briefly
                    // for its cooperative cancellation before the worker can
                    // promote a successor, otherwise stale/partial input
                    // could be consumed by that next generation.
                    approval_cancellation.cancel();
                    let _ = receiver.recv_timeout(Duration::from_millis(500));
                    run.pending_recoveries.append(&mut pending);
                    return true;
                }
                let remaining = deadline.saturating_duration_since(Instant::now());
                if remaining.is_zero() {
                    approval_cancellation.cancel();
                    let _ = receiver.recv_timeout(Duration::from_millis(500));
                    break ApprovalDecision::TimedOut;
                }
                match receiver.recv_timeout(POLL_INTERVAL.min(remaining)) {
                    Ok(decision) => break decision,
                    Err(std::sync::mpsc::RecvTimeoutError::Timeout) => continue,
                    Err(std::sync::mpsc::RecvTimeoutError::Disconnected) => {
                        break ApprovalDecision::Invalid
                    }
                }
            }
        } else {
            ApprovalDecision::Declined
        };
        if matches!(decision, ApprovalDecision::Cancelled) || run.cancellation_requested() {
            run.pending_recoveries.append(&mut pending);
            return true;
        }
        if !matches!(decision, ApprovalDecision::Approved) {
            let reason = match (run.metadata.recovery_policy, decision) {
                (crate::config::RecoveryPolicy::Skip, _) => "recovery_policy: skip",
                (_, ApprovalDecision::NoTty) => "no TTY",
                (_, ApprovalDecision::Eof) => "EOF",
                (_, ApprovalDecision::Invalid) => "invalid answer",
                (_, ApprovalDecision::Declined) => "declined",
                (_, ApprovalDecision::TimedOut) => "approval timeout",
                (_, ApprovalDecision::Cancelled) => "cancelled",
                (_, ApprovalDecision::Approved) => unreachable!(),
            };
            stdout::warn(&format!(
                "Recovery for job(s) {} was not run: {reason}",
                requests
                    .iter()
                    .map(|request| request.job.as_str())
                    .collect::<Vec<_>>()
                    .join(", ")
            ));
            for task in pending.drain(..) {
                self.events.emit(Event::RecoveryPhase {
                    run_id: run.metadata.run_id,
                    job: task.name.clone(),
                    phase: "approval_decided".to_owned(),
                    outcome: Some(reason.to_owned()),
                });
                run.results.push(Err(task
                    .failures
                    .first()
                    .cloned()
                    .unwrap_or_else(|| format!("Job '{}' failed", task.name))));
                self.record_task_outcome(run, task);
            }
            return false;
        }

        let mut remaining = pending.into_iter();
        while let Some(mut task) = remaining.next() {
            self.events.emit(Event::RecoveryPhase {
                run_id: run.metadata.run_id,
                job: task.name.clone(),
                phase: "approval_decided".to_owned(),
                outcome: Some("approved".to_owned()),
            });
            let original_failures = task.failures.clone();
            let Some(recovery_commands) = task.recovery_commands.take() else {
                self.record_task_outcome(run, task);
                continue;
            };
            self.events.emit(Event::RecoveryPhase {
                run_id: run.metadata.run_id,
                job: task.name.clone(),
                phase: "recovery_started".to_owned(),
                outcome: None,
            });
            task.commands = recovery_commands;
            task.failures.clear();
            task.current_command = None;
            task.child = None;
            task.command_index = 0;
            task.command_total = task.commands.len();
            task.defer_failure = false;
            let mut recovery_results = vec![];
            while matches!(
                self.advance_task(&mut task, &mut recovery_results, run.metadata.run_id, true,),
                TaskStep::Running
            ) {
                if run.cancellation_requested() {
                    self.shutdown_task(&mut task);
                    run.pending_recoveries.push(task);
                    run.pending_recoveries.extend(remaining);
                    return true;
                }
                self.clock.sleep(POLL_INTERVAL);
            }
            let recovery_failed = !task.failures.is_empty();
            self.events.emit(Event::RecoveryPhase {
                run_id: run.metadata.run_id,
                job: task.name.clone(),
                phase: "recovery_finished".to_owned(),
                outcome: Some(if recovery_failed { "failed" } else { "passed" }.to_owned()),
            });
            if recovery_failed {
                let mut failures = original_failures;
                failures.extend(task.failures.clone());
                task.failures = failures;
                run.results.push(Err(task
                    .failures
                    .last()
                    .cloned()
                    .unwrap_or_else(|| format!("Recovery failed for job '{}'", task.name))));
                self.record_task_outcome(run, task);
                for remaining in remaining {
                    run.results.push(Err(remaining
                        .failures
                        .first()
                        .cloned()
                        .unwrap_or_else(|| format!("Job '{}' failed", remaining.name))));
                    self.record_task_outcome(run, remaining);
                }
                return false;
            }

            self.events.emit(Event::RecoveryPhase {
                run_id: run.metadata.run_id,
                job: task.name.clone(),
                phase: "verification_started".to_owned(),
                outcome: None,
            });
            task.commands = task.original_commands.clone().into();
            task.failures.clear();
            task.current_command = None;
            task.child = None;
            task.command_index = 0;
            task.command_total = task.commands.len();
            task.defer_failure = false;
            let mut verification_results = vec![];
            while matches!(
                self.advance_task(
                    &mut task,
                    &mut verification_results,
                    run.metadata.run_id,
                    true,
                ),
                TaskStep::Running
            ) {
                if run.cancellation_requested() {
                    self.shutdown_task(&mut task);
                    run.pending_recoveries.push(task);
                    run.pending_recoveries.extend(remaining);
                    return true;
                }
                self.clock.sleep(POLL_INTERVAL);
            }
            let verification_failed = !task.failures.is_empty();
            self.events.emit(Event::RecoveryPhase {
                run_id: run.metadata.run_id,
                job: task.name.clone(),
                phase: "verification_finished".to_owned(),
                outcome: Some(
                    if verification_failed {
                        "failed"
                    } else {
                        "passed"
                    }
                    .to_owned(),
                ),
            });
            if verification_failed {
                run.results
                    .push(Err(task.failures.last().cloned().unwrap_or_else(|| {
                        format!("Verification failed for job '{}'", task.name)
                    })));
            } else {
                run.results.push(Ok(()));
            }
            self.record_task_outcome(run, task);
        }
        false
    }

    fn record_task_outcome(&self, run: &mut Run, task: ActiveTask) {
        self.reveal_on_failure(&task, &task.failures);
        if let (Some(outputs), Some(capture)) = (&self.outputs, &task.capture) {
            outputs.record(
                run.metadata.run_id,
                task.name.clone(),
                capture.finish(),
                run.metadata.revision,
                run.metadata.revision_hash.clone(),
            );
        }
        let duration_ms = task
            .started
            .map(|started| self.clock.elapsed(started).as_millis() as u64);
        let timed_out = task.timed_out;
        let (state, outcome) = if task.failures.is_empty() {
            (TaskState::Passed, TaskOutcome::Passed)
        } else if timed_out {
            // FINITE-JOB-TIMEOUT-CONTRACT §4: a timeout is typed distinctly
            // from command failure at the task-snapshot surface; the
            // generation still fails.
            let failures = task.failures.clone();
            (TaskState::TimedOut, TaskOutcome::Failed { failures })
        } else {
            let failures = task.failures.clone();
            (TaskState::Failed, TaskOutcome::Failed { failures })
        };
        self.record_task_snapshot(
            run,
            task.position,
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

    /// FINITE-JOB-TIMEOUT-CONTRACT §4–§7: terminate the task's process
    /// group, record the typed timeout failure, and mark the task timed out.
    /// Shared by the pre-try_wait deadline check and the pre-spawn
    /// sequential recheck so both paths produce identical outcomes.
    fn expire_task(
        &self,
        task: &mut ActiveTask,
        results: &mut Vec<Result<(), String>>,
    ) -> TaskStep {
        let elapsed = task
            .started
            .map(|started| self.clock.elapsed(started))
            .unwrap_or_default();
        // Full process-group termination with graceful shutdown and
        // escalation (§5), reusing the existing ownership.
        self.shutdown_task(task);
        let failure = format!(
            "job '{}' timed out after {:?} and was terminated",
            task.name,
            elapsed.max(Duration::from_millis(0))
        );
        task.failures.push(failure);
        results.push(Err(format!(
            "Job '{}' timed out after {}ms and was terminated",
            task.name,
            elapsed.as_millis()
        )));
        // §7: a timed-out job never enters pending_recoveries — recovery
        // targets command failures, not deadlines.
        task.recovery_commands = None;
        task.timed_out = true;
        TaskStep::TimedOut
    }

    fn stop_after_failure(&self, run: &mut Run) {
        for mut task in std::mem::take(&mut run.active) {
            self.shutdown_task(&mut task);
            if let (Some(outputs), Some(capture)) = (&self.outputs, &task.capture) {
                outputs.record(
                    run.metadata.run_id,
                    task.name.clone(),
                    capture.finish(),
                    run.metadata.revision,
                    run.metadata.revision_hash.clone(),
                );
            }
            let duration_ms = task
                .started
                .map(|started| self.clock.elapsed(started).as_millis() as u64);
            self.record_task_snapshot(
                run,
                task.position,
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
        for task in std::mem::take(&mut run.queued) {
            self.record_task_snapshot(
                run,
                task.position,
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
        for stage in std::mem::take(&mut run.stages) {
            for task in stage_tasks(stage) {
                self.record_task_snapshot(
                    run,
                    task.position,
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
                    !matches!(service.output, crate::rules::OutputPolicy::Inherit),
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
                        let duration_ms = done
                            .started
                            .map(|started| self.clock.elapsed(started).as_millis() as u64);
                        self.record_task_snapshot(
                            run,
                            done.position,
                            &done.name,
                            done.group_occurrence.as_deref(),
                            TaskState::Failed,
                            duration_ms,
                        );
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
                        let duration_ms = done
                            .started
                            .map(|started| self.clock.elapsed(started).as_millis() as u64);
                        self.record_task_snapshot(
                            run,
                            done.position,
                            &done.name,
                            done.group_occurrence.as_deref(),
                            TaskState::Passed,
                            duration_ms,
                        );
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
                    let duration_ms = done
                        .started
                        .map(|started| self.clock.elapsed(started).as_millis() as u64);
                    self.record_task_snapshot(
                        run,
                        done.position,
                        &done.name,
                        done.group_occurrence.as_deref(),
                        TaskState::Failed,
                        duration_ms,
                    );
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
    fn run_terminal_hook(&self, metadata: &RunMetadata, outcome: &RunOutcome) -> Option<PendingSettledHook> {
        let command = if outcome.is_success() {
            metadata.hooks.success.as_deref()
        } else {
            metadata.hooks.failure.as_deref()
        };
        let Some(command) = command else { return None };
        if !outcome.is_success() {
            if let Some(settle) = metadata.hooks.failure_settle {
                return Some(PendingSettledHook {
                    run_id: metadata.run_id,
                    command: command.to_owned(),
                    settle,
                    revision: metadata.revision.unwrap_or(0),
                    revision_hash: metadata.revision_hash.clone().unwrap_or_default(),
                });
            }
        }
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
        None
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
        let pending_settled_hook = self.run_terminal_hook(&run.metadata, &outcome);
        self.events.emit(Event::Finished {
            run_id: run.metadata.run_id,
            superseded_by: run.superseded_by,
            elapsed,
            failures: failures.clone(),
        });
        run.task_snapshots.sort_by_key(|task| task.position);
        CompletedRun {
            results: run.results,
            elapsed,
            outcome,
            tasks: run.task_snapshots,
            pending_settled_hook,
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
        for mut task in std::mem::take(&mut run.active) {
            if self.shutdown_task(&mut task) {
                escalated = true;
            }
            if let (Some(outputs), Some(capture)) = (&self.outputs, &task.capture) {
                outputs.record(
                    run.metadata.run_id,
                    task.name.clone(),
                    capture.finish(),
                    run.metadata.revision,
                    run.metadata.revision_hash.clone(),
                );
            }
            let duration_ms = task
                .started
                .map(|started| self.clock.elapsed(started).as_millis() as u64);
            self.record_task_snapshot(
                run,
                task.position,
                &task.name,
                task.group_occurrence.as_deref(),
                TaskState::Cancelled,
                duration_ms,
            );
        }
        for mut task in std::mem::take(&mut run.services) {
            if self.shutdown_task(&mut task) {
                escalated = true;
            }
            if let (Some(outputs), Some(capture)) = (&self.outputs, &task.capture) {
                outputs.record(
                    run.metadata.run_id,
                    task.name.clone(),
                    capture.finish(),
                    run.metadata.revision,
                    run.metadata.revision_hash.clone(),
                );
            }
            let duration_ms = task
                .started
                .map(|started| self.clock.elapsed(started).as_millis() as u64);
            self.record_task_snapshot(
                run,
                task.position,
                &task.name,
                task.group_occurrence.as_deref(),
                TaskState::Cancelled,
                duration_ms,
            );
        }
        for mut task in std::mem::take(&mut run.pending_recoveries) {
            if self.shutdown_task(&mut task) {
                escalated = true;
            }
            self.record_task_snapshot(
                run,
                task.position,
                &task.name,
                task.group_occurrence.as_deref(),
                TaskState::Cancelled,
                task.started
                    .map(|started| self.clock.elapsed(started).as_millis() as u64),
            );
        }
        for task in std::mem::take(&mut run.queued) {
            self.record_task_snapshot(
                run,
                task.position,
                &task.name,
                task.group_occurrence.as_deref(),
                TaskState::Cancelled,
                None,
            );
        }
        for stage in std::mem::take(&mut run.stages) {
            for task in stage_tasks(stage) {
                self.record_task_snapshot(
                    run,
                    task.position,
                    &task.name,
                    task.group_occurrence.as_deref(),
                    TaskState::Cancelled,
                    None,
                );
            }
        }
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

    /// Reconciles the background services of one generation by name
    /// (TASK-0090 AC6): a service whose name is in `stop_names` is gracefully
    /// stopped (bounded kill/reap via the ownership path) and removed from
    /// the background pool. Services not named remain owned and untouched.
    /// Returns the names still running after reconciliation.
    ///
    /// The caller (reload transaction) computes the stop set from the
    /// revision diff (removed services + signature-changed services) and
    /// starts the new/changed services under the new revision; this method
    /// never spawns — it only retires.
    /// Appends a plan's stages to a RUNNING generation (TASK-0090 AC6): used
    /// by the reload transaction to start new/changed managed services inside
    /// the active generation without replacing it — active finite work and
    /// unchanged services stay untouched (contract §4: a config save alone
    /// never kills an active generation). The appended service tasks are
    /// spawned by the next `advance` and moved to the background pool.
    pub fn append_plan(&self, run: &mut Run, plan: RunPlan) {
        for stage in plan.stages {
            run.stages.push_back(stage);
        }
    }

    /// Reconciles the background services of one generation by name
    /// (TASK-0090 AC6): a service whose name is in `stop_names` is gracefully
    /// stopped (bounded kill/reap via the ownership path) and removed from
    /// the background pool. Services not named remain owned and untouched.
    /// Returns the names still running after reconciliation.
    ///
    /// The caller (reload transaction) computes the stop set from the
    /// revision diff (removed services + signature-changed services) and
    /// starts the new/changed services under the new revision; this method
    /// never spawns — it only retires.
    pub fn reconcile_services(&self, run: &mut Run, stop_names: &[&str]) -> Vec<String> {
        let mut index = 0;
        let mut still_running = vec![];
        while index < run.services.len() {
            let service = &run.services[index];
            if stop_names.contains(&service.name.as_str()) {
                // Graceful, bounded stop of a changed/removed service.
                let mut removed = run.services.remove(index);
                self.shutdown_task(&mut removed);
                run.outcomes.push((
                    removed.position,
                    removed.name.clone(),
                    removed.group_occurrence.clone(),
                    TaskOutcome::Cancelled,
                ));
                continue;
            }
            still_running.push(service.name.clone());
            index += 1;
        }
        still_running
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

    #[derive(Clone)]
    struct TestApproval {
        decision: ApprovalDecision,
        requests: Arc<Mutex<Vec<RecoveryRequest>>>,
    }

    impl RecoveryApproval for TestApproval {
        fn approve(
            &self,
            requests: &[RecoveryRequest],
            cancellation: &CancellationToken,
            _timeout: Duration,
        ) -> ApprovalDecision {
            self.requests.lock().unwrap().extend_from_slice(requests);
            if cancellation.is_cancelled() {
                ApprovalDecision::Cancelled
            } else {
                self.decision
            }
        }
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

    fn recovery_rule(run: &str, recovery: &[&str]) -> Rules {
        named_recovery_rule("recoverable", run, recovery)
    }

    fn named_recovery_rule(name: &str, run: &str, recovery: &[&str]) -> Rules {
        Rules::new(name.to_owned(), vec![run.to_owned()], vec![], vec![], true).with_recovery(
            recovery
                .iter()
                .map(|command| (*command).to_owned())
                .collect(),
        )
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
                recovery_commands: None,
                parallel: None,
                group_occurrence: None,
                rule,
                context: crate::plan::TaskContext::default(),
                output: crate::rules::OutputPolicy::Inherit,
                service: false,
                timeout: None,
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
    fn approved_recovery_runs_once_then_verifies_original_job() {
        let marker =
            std::env::temp_dir().join(format!("funzzy-recovery-{}-{}", std::process::id(), 1));
        let _ = std::fs::remove_file(&marker);
        let path = marker.display().to_string();
        let requests = Arc::new(Mutex::new(Vec::new()));
        let approval = TestApproval {
            decision: ApprovalDecision::Approved,
            requests: Arc::clone(&requests),
        };
        let executor = Executor::new(
            Arc::new(SystemProcessRunner),
            Arc::new(SystemClock),
            1,
            Arc::new(|_| {}),
            false,
            false,
        )
        .unwrap()
        .with_recovery_approval(Arc::new(approval));
        let completed = executor.run_to_completion(
            RunMetadata::new(42, "test").with_revision(9, "revision-hash".to_owned()),
            RunPlan::from_rules(vec![recovery_rule(
                &format!("test -f '{path}'"),
                &[&format!("touch '{path}'")],
            )]),
        );
        assert!(completed.outcome.is_success());
        assert_eq!(completed.tasks.len(), 1);
        assert_eq!(completed.tasks[0].state, TaskState::Passed);
        assert!(completed.tasks[0].duration_ms.is_some());
        assert_eq!(requests.lock().unwrap()[0].generation, 42);
        assert_eq!(requests.lock().unwrap()[0].revision, Some(9));
        assert_eq!(requests.lock().unwrap()[0].job_position, 0);
        assert_eq!(
            requests.lock().unwrap()[0].commands,
            vec![format!("touch '{path}'")]
        );
        let _ = std::fs::remove_file(marker);
    }

    #[test]
    fn recovery_lifecycle_emits_phases_and_one_terminal_task_event() {
        let events = Arc::new(Mutex::new(Vec::new()));
        let event_sink = Arc::clone(&events);
        let requests = Arc::new(Mutex::new(Vec::new()));
        let executor = Executor::new(
            Arc::new(SystemProcessRunner),
            Arc::new(SystemClock),
            1,
            Arc::new(move |event| event_sink.lock().unwrap().push(event)),
            false,
            false,
        )
        .unwrap()
        .with_recovery_approval(Arc::new(TestApproval {
            decision: ApprovalDecision::Approved,
            requests,
        }));
        let completed = executor.run_to_completion(
            RunMetadata::new(48, "test"),
            RunPlan::from_rules(vec![recovery_rule("false", &["true"])]),
        );
        assert!(!completed.outcome.is_success());
        let events = events.lock().unwrap();
        let phases = events
            .iter()
            .filter_map(|event| match event {
                Event::RecoveryPhase { phase, .. } => Some(phase.as_str()),
                _ => None,
            })
            .collect::<Vec<_>>();
        assert_eq!(
            phases,
            [
                "original_failed",
                "approval_requested",
                "approval_decided",
                "recovery_started",
                "recovery_finished",
                "verification_started",
                "verification_finished",
            ]
        );
        assert_eq!(
            events
                .iter()
                .filter(|event| matches!(event, Event::TaskTerminal { .. }))
                .count(),
            1
        );
    }

    #[test]
    fn parallel_recovery_requests_follow_declaration_order() {
        let requests = Arc::new(Mutex::new(Vec::new()));
        let executor = Executor::new(
            Arc::new(SystemProcessRunner),
            Arc::new(SystemClock),
            2,
            Arc::new(|_| {}),
            false,
            false,
        )
        .unwrap()
        .with_recovery_approval(Arc::new(TestApproval {
            decision: ApprovalDecision::Approved,
            requests: Arc::clone(&requests),
        }));
        let completed = executor.run_to_completion(
            RunMetadata::new(47, "test"),
            RunPlan::from_rules(vec![
                named_recovery_rule("first", "false", &["true"]),
                named_recovery_rule("second", "false", &["true"]),
            ]),
        );
        let requests = requests.lock().unwrap();
        assert_eq!(
            requests
                .iter()
                .map(|request| request.job.as_str())
                .collect::<Vec<_>>(),
            ["first", "second"]
        );
        assert!(!completed.outcome.is_success());
    }

    #[test]
    fn skip_policy_preserves_failure_without_spawning_recovery() {
        let marker =
            std::env::temp_dir().join(format!("funzzy-recovery-skip-{}", std::process::id()));
        let _ = std::fs::remove_file(&marker);
        let path = marker.display().to_string();
        let executor = Executor::new(
            Arc::new(SystemProcessRunner),
            Arc::new(SystemClock),
            1,
            Arc::new(|_| {}),
            false,
            false,
        )
        .unwrap();
        let completed = executor.run_to_completion(
            RunMetadata::new(43, "test").with_recovery_policy(crate::config::RecoveryPolicy::Skip),
            RunPlan::from_rules(vec![recovery_rule("false", &[&format!("touch '{path}'")])]),
        );
        assert!(!completed.outcome.is_success());
        assert!(!marker.exists());
    }

    #[test]
    fn declined_recovery_preserves_original_failure() {
        let requests = Arc::new(Mutex::new(Vec::new()));
        let executor = Executor::new(
            Arc::new(SystemProcessRunner),
            Arc::new(SystemClock),
            1,
            Arc::new(|_| {}),
            false,
            false,
        )
        .unwrap()
        .with_recovery_approval(Arc::new(TestApproval {
            decision: ApprovalDecision::Declined,
            requests,
        }));
        let completed = executor.run_to_completion(
            RunMetadata::new(44, "test"),
            RunPlan::from_rules(vec![recovery_rule("false", &["true"])]),
        );
        assert!(!completed.outcome.is_success());
        assert!(completed.results.iter().any(Result::is_err));
    }

    #[test]
    fn timed_out_recovery_preserves_failure_without_running_recovery() {
        let marker =
            std::env::temp_dir().join(format!("funzzy-recovery-timeout-{}", std::process::id()));
        let _ = std::fs::remove_file(&marker);
        let requests = Arc::new(Mutex::new(Vec::new()));
        let executor = Executor::new(
            Arc::new(SystemProcessRunner),
            Arc::new(SystemClock),
            1,
            Arc::new(|_| {}),
            false,
            false,
        )
        .unwrap()
        .with_recovery_approval(Arc::new(TestApproval {
            decision: ApprovalDecision::TimedOut,
            requests,
        }));
        let completed = executor.run_to_completion(
            RunMetadata::new(45, "test").with_recovery_timeout(Duration::from_millis(1)),
            RunPlan::from_rules(vec![recovery_rule(
                "false",
                &[&format!("touch '{}'", marker.display())],
            )]),
        );
        assert!(!completed.outcome.is_success());
        assert!(!marker.exists());
    }

    #[test]
    fn recovery_failure_and_verification_failure_are_final_failures() {
        let approval = Arc::new(TestApproval {
            decision: ApprovalDecision::Approved,
            requests: Arc::new(Mutex::new(Vec::new())),
        });
        let executor = Executor::new(
            Arc::new(SystemProcessRunner),
            Arc::new(SystemClock),
            1,
            Arc::new(|_| {}),
            false,
            false,
        )
        .unwrap()
        .with_recovery_approval(approval);
        let failed_recovery = executor.run_to_completion(
            RunMetadata::new(45, "test"),
            RunPlan::from_rules(vec![recovery_rule("false", &["false"])]),
        );
        assert!(!failed_recovery.outcome.is_success());

        let verification_failure = executor.run_to_completion(
            RunMetadata::new(46, "test"),
            RunPlan::from_rules(vec![recovery_rule("false", &["true"])]),
        );
        assert!(!verification_failure.outcome.is_success());
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
    fn completed_run_carries_executor_snapshots_for_failed_and_skipped_jobs() {
        let executor = Executor::new(
            Arc::new(SystemProcessRunner),
            Arc::new(FixedClock),
            1,
            Arc::new(|_| {}),
            true,
            false,
        )
        .unwrap();
        let completed = executor.run_to_completion(
            RunMetadata::new(8, "test"),
            RunPlan::from_rules(vec![
                task("failed", None, &["false"]),
                task("skipped", None, &["true"]),
            ]),
        );

        assert_eq!(completed.tasks.len(), 2);
        assert_eq!(completed.tasks[0].name, "failed");
        assert_eq!(completed.tasks[0].state, TaskState::Failed);
        assert_eq!(completed.tasks[0].duration_ms, Some(42));
        assert_eq!(completed.tasks[1].name, "skipped");
        assert_eq!(completed.tasks[1].state, TaskState::Cancelled);
        assert_eq!(completed.tasks[1].duration_ms, None);
    }

    #[test]
    fn completed_run_sorts_parallel_snapshots_by_configured_position() {
        let runner = FakeRunner::default();
        let executor = Executor::new(
            Arc::new(runner.clone()),
            Arc::new(FixedClock),
            2,
            Arc::new(|_| {}),
            false,
            false,
        )
        .unwrap();
        let mut run = executor.start(
            RunMetadata::new(9, "test"),
            RunPlan::from_rules(vec![
                task("first", Some("checks"), &["first"]),
                task("second", Some("checks"), &["second"]),
            ]),
        );
        assert!(matches!(executor.advance(&mut run), Step::Running));
        runner.complete("second", true);
        assert!(matches!(executor.advance(&mut run), Step::Running));
        runner.complete("first", true);
        assert!(matches!(executor.advance(&mut run), Step::Finished));

        let completed = executor.finish(run);
        assert_eq!(
            completed
                .tasks
                .iter()
                .map(|task| task.name.as_str())
                .collect::<Vec<_>>(),
            ["first", "second"],
            "parallel completion order never changes report order"
        );
        assert!(completed
            .tasks
            .iter()
            .all(|task| task.duration_ms == Some(42)));
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

        fn shutdown_commands(&self) -> Vec<String> {
            self.state
                .lock()
                .unwrap()
                .shutdown
                .iter()
                .cloned()
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
        assert_eq!(
            completed
                .tasks
                .iter()
                .map(|task| (task.name.as_str(), task.state, task.duration_ms))
                .collect::<Vec<_>>(),
            [
                ("A", TaskState::Failed, Some(42)),
                ("B", TaskState::Cancelled, Some(42)),
                ("C", TaskState::Cancelled, None),
                ("D", TaskState::Cancelled, None),
            ]
        );
        assert!(runner.state.lock().unwrap().shutdown.contains("b"));
        assert!(!runner.started_commands().contains(&"c".to_owned()));
        assert!(!runner.started_commands().contains(&"d".to_owned()));
    }

    #[test]
    fn concurrency_change_affects_only_newly_planned_generation() {
        // TASK-0090 AC7: swapping the shared bound must not resize a RUNNING
        // group (its stage_limit is frozen at plan time) while the next
        // generation plans under the new bound.
        let runner = FakeRunner::default();
        let executor = fake_executor(runner.clone(), 1, false);
        let plan = RunPlan::from_rules(vec![
            task("A", Some("g"), &["a"]),
            task("B", Some("g"), &["b"]),
        ]);
        let mut run = executor.start(RunMetadata::new(1, "test"), plan);
        executor.advance(&mut run);
        // With limit 1 the running group planned one slot; B is queued.
        assert_eq!(runner.state.lock().unwrap().active, 1);

        // Swap the bound to 3 while the group runs: the RUNNING group must
        // keep its frozen stage_limit (still one slot).
        executor.set_concurrency_limit(3);
        assert_eq!(executor.concurrency_limit(), 3);
        executor.advance(&mut run);
        assert!(
            runner.state.lock().unwrap().active <= 1,
            "running group must not be resized inconsistently"
        );

        // Finish the running generation so its tasks do not pollute the
        // fresh-generation accounting.
        runner.complete("a", true);
        while !matches!(executor.advance(&mut run), Step::Finished) {
            runner.complete("b", true);
        }
        let _ = executor.finish(run);

        // A NEW generation plans under the new bound: three tasks fill
        // three slots.
        let big = RunPlan::from_rules(vec![
            task("A", Some("g"), &["a"]),
            task("B", Some("g"), &["b"]),
            task("C", Some("g"), &["c"]),
        ]);
        let mut fresh = executor.start(RunMetadata::new(2, "test"), big);
        executor.advance(&mut fresh);
        assert_eq!(
            runner.state.lock().unwrap().active,
            3,
            "new generation must plan under the committed bound"
        );
    }

    #[test]
    fn reconcile_services_stops_named_services_and_keeps_others_owned() {
        // TASK-0090 AC6: reconcile retires only the named services (graceful,
        // bounded shutdown) and leaves unnamed ones owned. It never spawns.
        let runner = FakeRunner::default();
        let executor = fake_executor(runner.clone(), 2, false);
        let plan = RunPlan::from_rules(vec![
            task("svc-a", None, &["sa"]).with_service(true),
            task("svc-b", None, &["sb"]).with_service(true),
        ]);
        let mut run = executor.start(RunMetadata::new(1, "test"), plan);
        // Both services spawn and move to the background pool.
        while run.services.len() < 2 {
            executor.advance(&mut run);
        }
        assert_eq!(runner.started_commands().len(), 2);

        // Stop only svc-a: svc-b must stay owned and running.
        let still = executor.reconcile_services(&mut run, &["svc-a"]);
        assert_eq!(still, vec!["svc-b".to_owned()]);
        assert!(
            runner.state.lock().unwrap().shutdown.contains("sa"),
            "svc-a must be gracefully shut down"
        );
        assert!(
            !runner.state.lock().unwrap().shutdown.contains("sb"),
            "svc-b must stay owned"
        );
    }

    #[test]
    fn append_plan_starts_services_without_replacing_active_generation() {
        // TASK-0090 AC6: append_plan injects a service plan into the RUNNING
        // generation; the executor keeps running it (services keep the
        // generation alive) and the new service spawns into the background.
        let runner = FakeRunner::default();
        let executor = fake_executor(runner.clone(), 2, false);
        let plan =
            RunPlan::from_rules(vec![task("svc-a", Some("srv"), &["sa"]).with_service(true)]);
        let mut run = executor.start(RunMetadata::new(1, "test"), plan);
        executor.advance(&mut run);
        assert_eq!(runner.started_commands().len(), 1);

        // Reload adds svc-b: append it to the live generation.
        let addition =
            RunPlan::from_rules(vec![task("svc-b", Some("srv"), &["sb"]).with_service(true)]);
        executor.append_plan(&mut run, addition);
        executor.advance(&mut run);
        assert_eq!(
            runner.started_commands().len(),
            2,
            "new service must start without replacing the generation"
        );
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

    /// Deterministic wall clock: `now` advances only when the test moves it
    /// (explicitly via `advance_ms`, or virtually when the poll loop sleeps),
    /// so the deadline check is a pure function of test-driven state. The
    /// base is captured ONCE — never re-read from the real clock — otherwise
    /// real-time drift leaks into deadline comparisons (QA defect).
    struct ManualClock {
        base: Instant,
        elapsed_ms: std::sync::atomic::AtomicU64,
    }

    impl ManualClock {
        fn new() -> Self {
            Self {
                base: Instant::now(),
                elapsed_ms: std::sync::atomic::AtomicU64::new(0),
            }
        }

        fn advance_ms(&self, ms: u64) {
            self.elapsed_ms
                .fetch_add(ms, std::sync::atomic::Ordering::SeqCst);
        }
    }

    impl Clock for ManualClock {
        fn now(&self) -> Instant {
            // Monotonic virtual time: fixed base + injected offset. Instants
            // are only compared against deadlines minted from this clock.
            self.base
                + Duration::from_millis(self.elapsed_ms.load(std::sync::atomic::Ordering::SeqCst))
        }

        fn elapsed(&self, started: Instant) -> Duration {
            self.now().saturating_duration_since(started)
        }

        fn sleep(&self, duration: Duration) {
            // Virtual time: the poll loop's sleep advances the deterministic
            // clock, so deadlines elapse without real waits and exactly as
            // fast as the loop polls.
            self.elapsed_ms.fetch_add(
                duration.as_millis() as u64,
                std::sync::atomic::Ordering::SeqCst,
            );
        }
    }

    fn timed_task(name: &str, timeout_ms: u64, commands: &[&str]) -> Rules {
        Rules::new(
            name.to_owned(),
            commands.iter().map(|c| c.to_string()).collect(),
            vec!["src/**".to_owned()],
            vec![],
            false,
        )
        .with_timeout(Some(Duration::from_millis(timeout_ms)))
    }

    fn executor_with(runner: FakeRunner, clock: Arc<ManualClock>, fail_fast: bool) -> Executor {
        Executor::new(
            Arc::new(runner),
            clock,
            1,
            Arc::new(|_| {}),
            fail_fast,
            false,
        )
        .unwrap()
    }

    /// §4: the task snapshot is typed TimedOut, distinct from Failed, and
    /// the generation fails with the timeout message.
    #[test]
    fn timed_out_job_records_typed_state_and_fails_the_generation() {
        let runner = FakeRunner::default();
        let clock = Arc::new(ManualClock::new());
        let executor = executor_with(runner.clone(), clock, false);
        // The fake child never completes: only the deadline ends it.
        let completed = executor.run_to_completion(
            RunMetadata::new(101, "test"),
            RunPlan::from_rules(vec![timed_task("blocked", 100, &["never"])]),
        );
        assert!(!completed.outcome.is_success());
        assert_eq!(completed.tasks[0].state, TaskState::TimedOut);
        assert!(completed
            .results
            .iter()
            .any(|r| r.as_ref().unwrap_err().contains("timed out")));
    }

    /// §3 precedence: a natural exit before the deadline is a normal outcome.
    #[test]
    fn natural_exit_before_deadline_wins() {
        let runner = FakeRunner::default();
        runner.complete("quick", true);
        let executor = executor_with(runner.clone(), Arc::new(ManualClock::new()), false);
        let completed = executor.run_to_completion(
            RunMetadata::new(102, "test"),
            RunPlan::from_rules(vec![timed_task("quick", 60_000, &["quick"])]),
        );
        assert!(completed.outcome.is_success());
        assert_eq!(completed.tasks[0].state, TaskState::Passed);
    }

    /// §5: the whole process group is terminated through shutdown_task.
    #[test]
    fn timeout_terminates_the_process_group_via_shutdown() {
        let runner = FakeRunner::default();
        let executor = executor_with(runner.clone(), Arc::new(ManualClock::new()), false);
        let _ = executor.run_to_completion(
            RunMetadata::new(103, "test"),
            RunPlan::from_rules(vec![timed_task("blocked", 5, &["stuck"])]),
        );
        assert!(
            runner.shutdown_commands().contains(&"stuck".to_owned()),
            "shutdown was invoked on the timed-out child"
        );
    }

    /// §7: no recovery is offered after a timeout even when configured.
    #[test]
    fn timeout_never_enters_recovery_even_when_configured() {
        let runner = FakeRunner::default();
        let executor = executor_with(runner, Arc::new(ManualClock::new()), false);
        let rule = timed_task("guarded", 5, &["stuck"]).with_recovery(vec!["echo fix".to_owned()]);
        let completed = executor.run_to_completion(
            RunMetadata::new(104, "test"),
            RunPlan::from_rules(vec![rule]),
        );
        assert_eq!(completed.tasks[0].state, TaskState::TimedOut);
        assert!(!completed.outcome.is_success());
    }

    /// §3 job-wide rule: a two-command job gets ONE budget; the second
    /// command's spawn rechecks the ORIGINAL deadline (no reset). With the
    /// fake clock frozen at spawn, the deadline is `started + timeout`, and
    /// the timeout branch is reached only when `now` passes it — here the
    /// first command completes instantly and the clock is already past the
    /// whole budget at the second spawn check.
    #[test]
    fn sequential_commands_share_the_job_wide_budget() {
        let runner = FakeRunner::default();
        runner.complete("first", true);
        // 'second' never completes: the shared budget must already be spent.
        let executor = executor_with(runner.clone(), Arc::new(ManualClock::new()), false);
        let rule = timed_task("pair", 1, &["first", "second"])
            .with_timeout(Some(Duration::from_millis(1)));
        let completed = executor.run_to_completion(
            RunMetadata::new(105, "test"),
            RunPlan::from_rules(vec![rule]),
        );
        assert_eq!(
            completed.tasks[0].state,
            TaskState::TimedOut,
            "second command inherits the spent job budget, not a fresh one"
        );
        assert!(
            runner.started_commands().contains(&"first".to_owned()),
            "first command ran and completed naturally"
        );
        assert!(
            !runner.started_commands().contains(&"second".to_owned()),
            "the spent budget must not spawn the second command at all"
        );
    }

    /// §7 fail-fast: a timed-out job fails the generation and stops siblings
    /// exactly like a command failure.
    #[test]
    fn timeout_fail_fast_stops_parallel_siblings() {
        let runner = FakeRunner::default();
        runner.complete("sibling", true);
        let executor = executor_with(runner, Arc::new(ManualClock::new()), true);
        let completed = executor.run_to_completion(
            RunMetadata::new(106, "test"),
            RunPlan::from_rules(vec![
                timed_task("blocked", 1, &["stuck"]),
                timed_task("sibling", 60_000, &["sibling"]),
            ]),
        );
        assert!(!completed.outcome.is_success());
        assert!(completed
            .tasks
            .iter()
            .any(|t| t.state == TaskState::TimedOut));
    }

    /// The virtual clock is deterministic by construction: `now` moves only
    /// through `advance_ms` (tests) or `sleep` (poll loop), never real time.
    #[test]
    fn manual_clock_advances_only_when_moved() {
        let clock = ManualClock::new();
        let started = clock.now();
        assert_eq!(clock.elapsed(started), Duration::from_millis(0));
        clock.advance_ms(5);
        assert_eq!(clock.elapsed(started), Duration::from_millis(5));
        clock.sleep(Duration::from_millis(10));
        assert_eq!(clock.elapsed(started), Duration::from_millis(15));
    }

    /// §3 precedence #1 (QA coverage gap): the cancellation guard at the top
    /// of `advance()` outranks the deadline — with the budget already spent,
    /// a cancelled generation reports Cancelled, never TimedOut, and the
    /// timed-out path never evaluates.
    #[test]
    fn cancellation_wins_over_an_elapsed_deadline() {
        let runner = FakeRunner::default();
        let clock = Arc::new(ManualClock::new());
        let executor = executor_with(runner.clone(), clock.clone(), false);
        let mut run = executor.start(
            RunMetadata::new(109, "test"),
            RunPlan::from_rules(vec![timed_task("blocked", 5, &["stuck"])]),
        );
        executor.advance(&mut run); // spawns "stuck" at t=0

        // Cancellation is requested FIRST, then the deadline elapses.
        run.cancellation_token().cancel();
        clock.advance_ms(50); // deadline (5ms) is spent

        // The guard holds: advance never takes the timeout path.
        assert!(matches!(executor.advance(&mut run), Step::Running));
        executor.cancel(&mut run, None); // the worker's token reap
        let finished = executor.finish(run);
        assert_eq!(
            finished.tasks[0].state,
            TaskState::Cancelled,
            "cancellation must outrank the elapsed deadline"
        );
        assert!(
            !finished
                .results
                .iter()
                .any(|r| r.as_ref().is_err_and(|e| e.contains("timed out"))),
            "no timeout failure may be recorded for a cancelled run"
        );
    }

    /// §3 single ordering rule (QA coverage gap): the deadline check
    /// precedes try_wait in the same iteration — a child that already exited
    /// but was not yet reaped when the budget elapsed is a timeout outcome
    /// (outcome indeterminism bounded by one poll interval, accepted).
    #[test]
    fn elapsed_deadline_wins_over_an_unreaped_exit() {
        let runner = FakeRunner::default();
        // The fake child HAS exited — but the poll loop's sleep crosses the
        // deadline before try_wait reaps it.
        runner.complete("quick", true);
        let executor = executor_with(runner.clone(), Arc::new(ManualClock::new()), false);
        let completed = executor.run_to_completion(
            RunMetadata::new(110, "test"),
            RunPlan::from_rules(vec![timed_task("quick", 1, &["quick"])]),
        );
        assert_eq!(
            completed.tasks[0].state,
            TaskState::TimedOut,
            "deadline check runs before try_wait in the same iteration"
        );
    }

    /// §3 sequential recheck (QA defect): the ORIGINAL deadline is rechecked
    /// BEFORE a continuation spawn — an already-expired job budget must not
    /// start the next command at all (it would be born dead and killed one
    /// poll later).
    #[test]
    fn expired_sequential_continuation_is_never_spawned() {
        let runner = FakeRunner::default();
        runner.complete("first", true);
        let executor = executor_with(runner.clone(), Arc::new(ManualClock::new()), false);
        let rule = timed_task("pair", 5, &["first", "second"]);
        let completed = executor.run_to_completion(
            RunMetadata::new(107, "test"),
            RunPlan::from_rules(vec![rule]),
        );
        assert_eq!(
            completed.tasks[0].state,
            TaskState::TimedOut,
            "the spent job budget must end the job as a timeout"
        );
        assert!(
            runner.started_commands().contains(&"first".to_owned()),
            "first command ran and completed naturally"
        );
        assert!(
            !runner.started_commands().contains(&"second".to_owned()),
            "an expired budget must never spawn the continuation (§3 recheck-before-spawn)"
        );
    }

    /// §7 fail-fast symmetry (QA defect): without fail_fast, a timed-out job
    /// behaves exactly like a command failure — the running sibling keeps its
    /// own deadline and completes naturally; the generation still fails.
    #[test]
    fn timeout_without_fail_fast_lets_a_running_sibling_finish() {
        let runner = FakeRunner::default();
        runner.complete("sibling", true);
        let executor = Executor::new(
            Arc::new(runner.clone()),
            Arc::new(ManualClock::new()),
            2,
            Arc::new(|_| {}),
            false,
            false,
        )
        .unwrap();
        let completed = executor.run_to_completion(
            RunMetadata::new(108, "test"),
            RunPlan::from_rules(vec![
                timed_task("blocked", 1, &["stuck"]).with_parallel("obs".to_owned()),
                timed_task("sibling", 60_000, &["sibling"]).with_parallel("obs".to_owned()),
            ]),
        );
        assert!(
            !completed.outcome.is_success(),
            "timeout fails the generation"
        );
        let blocked = completed
            .tasks
            .iter()
            .find(|t| t.name == "blocked")
            .unwrap();
        assert_eq!(blocked.state, TaskState::TimedOut);
        let sibling = completed
            .tasks
            .iter()
            .find(|t| t.name == "sibling")
            .unwrap();
        assert_eq!(
            sibling.state,
            TaskState::Passed,
            "without fail_fast the sibling must not be cancelled by the timeout"
        );
    }
}
