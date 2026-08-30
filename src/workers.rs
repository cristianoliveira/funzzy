use crate::executor::{
    CancelDisposition, Event, EventSink, Executor, Run, RunMetadata, Step, SystemClock,
    SystemProcessRunner,
};
use crate::output::OutputRegistry;
use crate::plan::{ExecutionSignature, RunPlan};
use crate::rules::Rules;
use crate::stdout;
use crate::template::TemplateOptions;
use std::collections::{HashMap, VecDeque};
use std::path::PathBuf;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, Condvar, Mutex};
use std::thread::JoinHandle;
use std::time::{Duration, Instant};

enum SettlementState {
    Idle,
    Pending {
        spec: crate::executor::PendingSettledHook,
        deadline: Instant,
    },
    Claimed {
        spec: crate::executor::PendingSettledHook,
        token: crate::executor::CancellationToken,
    },
}
impl Default for SettlementState {
    fn default() -> Self {
        Self::new()
    }
}
impl SettlementState {
    fn new() -> Self {
        Self::Idle
    }
    fn register(&mut self, spec: crate::executor::PendingSettledHook, now: Instant) {
        if let Self::Claimed { token, .. } = self {
            token.cancel();
        }
        *self = Self::Pending {
            deadline: now + spec.settle,
            spec,
        };
    }
    fn newer_generation(&mut self) {
        if let Self::Claimed { token, .. } = self {
            token.cancel();
        }
        *self = Self::Idle;
    }
    fn claim_due(
        &mut self,
        now: Instant,
    ) -> Option<(
        crate::executor::PendingSettledHook,
        crate::executor::CancellationToken,
    )> {
        let Self::Pending { spec, deadline } = self else {
            return None;
        };
        if now < *deadline {
            return None;
        }
        let token = crate::executor::CancellationToken::new();
        let claim = (spec.clone(), token.clone());
        *self = Self::Claimed {
            spec: spec.clone(),
            token,
        };
        Some(claim)
    }
    fn shutdown(&mut self) {
        self.newer_generation();
    }
    fn deadline(&self) -> Option<Instant> {
        match self {
            Self::Pending { deadline, .. } => Some(*deadline),
            _ => None,
        }
    }
}

/// A run requested through the worker's command stream.
struct RunRequest {
    run_id: u64,
    plan: RunPlan,
    trigger: String,
    /// Debounce batch identity, when scheduled from a filesystem batch.
    batch: Option<u64>,
    /// Complete normalized changed-path set of the triggering batch.
    changed: Vec<String>,
    /// Generation identity this request replaces; set when it supersedes an
    /// active run (restart policy), so the relation survives to start.
    predecessor: Option<u64>,
    /// Exact configured target name (TASK-0054); None for fs/init/emit runs.
    target: Option<String>,
    /// Stable execution signature of the resolved plan (TASK-0054).
    execution_signature: Option<ExecutionSignature>,
    /// Per-generation effective concurrency (TASK-0073): Some(1) for a
    /// sequential control run; None keeps the worker's configured bound.
    effective_concurrency: Option<usize>,
    /// Override source label (TASK-0073): "control" when the generation was
    /// explicitly requested sequential over the control socket.
    concurrency_source: Option<&'static str>,
    /// Run-level terminal hooks (TASK-0040).
    hooks: crate::config::GenerationHooks,
    /// Frozen recovery policy for this request.
    recovery_policy: crate::config::RecoveryPolicy,
    recovery_timeout: Duration,
    /// Immutable config revision this request is frozen under (TASK-0089).
    revision: Option<u64>,
    /// Non-secret semantic hash of the frozen config revision.
    revision_hash: Option<String>,
}

/// Result of an exact-generation cancel (TASK-0046): the generation matched
/// (active or queued) and its termination disposition, or nothing matched.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum CancelResult {
    /// The generation was cancelled. The frozen config revision it ran under
    /// (TASK-0091, AC2) is reported additively so clients can attribute the
    /// cancellation to the exact revision; None for legacy runs.
    Cancelled {
        disposition: CancelDisposition,
        revision: Option<u64>,
        revision_hash: Option<String>,
    },
    Noop,
}

/// Scheduling, replacement, and cancellation flow through one ordered stream,
/// so a newer run always supersedes queued work and an exact cancel is a
/// compare-and-act on generation identity instead of a race with a separate
/// channel.
#[allow(clippy::large_enum_variant)]
enum WorkerCommand {
    Run(RunRequest),
    Cancel {
        /// `Some(id)` cancels only the exact generation; `None` cancels
        /// whatever is active (replacement/shutdown path).
        generation: Option<u64>,
        /// Reply channel for exact cancels, so the server can report whether
        /// the generation actually matched instead of guessing.
        reply: Option<std::sync::mpsc::Sender<CancelResult>>,
    },
    /// TASK-0090 AC6: gracefully stop the named managed services owned by the
    /// active generation (changed/removed by the reloaded revision) and
    /// report the names still running (unchanged services stay owned).
    ReconcileServices {
        stop_names: Vec<String>,
        reply: std::sync::mpsc::Sender<Vec<String>>,
    },
    /// TASK-0090 AC6: append a service-only plan to the ACTIVE generation so
    /// new/changed services start under the new revision WITHOUT replacing
    /// the active run (active finite work and unchanged services stay
    /// owned). No-op reply; errors surface through the generation outcome.
    StartServices {
        run_id: u64,
        plan: RunPlan,
        revision: Option<crate::config_revision::ConfigRevision>,
    },
}

/// Ordered command queue: at most one queued Run (newest wins), while
/// cancels are applied in order. The condition variable blocks idle consumers
/// without polling.
#[derive(Default)]
struct SchedulerState {
    queue: VecDeque<WorkerCommand>,
    closed: bool,
    settlement: SettlementState,
}

/// Scheduler that reports discarded queued work (contract §1): every
/// generation superseded before spawn gets a terminal Cancelled event with its
/// successor identity, so exact-generation awaits never hang.
enum SchedulerWake {
    Command(WorkerCommand),
    Timeout,
    SettlementDue(
        crate::executor::PendingSettledHook,
        crate::executor::CancellationToken,
    ),
    Closed,
}

struct Scheduler {
    state: Mutex<SchedulerState>,
    ready: Condvar,
    events: Arc<dyn EventSink>,
    active_cancellations: Mutex<HashMap<u64, crate::executor::CancellationToken>>,
}

impl Scheduler {
    fn new(events: Arc<dyn EventSink>) -> Self {
        Self {
            state: Mutex::new(SchedulerState::default()),
            ready: Condvar::new(),
            events,
            active_cancellations: Mutex::new(HashMap::new()),
        }
    }

    fn register_settlement(&self, spec: crate::executor::PendingSettledHook, now: Instant) {
        self.state.lock().unwrap().settlement.register(spec, now);
        self.ready.notify_all();
    }

    fn claim_settlement(
        &self,
        now: Instant,
    ) -> Option<(
        crate::executor::PendingSettledHook,
        crate::executor::CancellationToken,
    )> {
        self.state.lock().unwrap().settlement.claim_due(now)
    }

    fn cancel_settlement(&self) {
        self.state.lock().unwrap().settlement.newer_generation();
    }

    fn register_active(&self, run_id: u64, token: crate::executor::CancellationToken) {
        self.active_cancellations
            .lock()
            .unwrap()
            .insert(run_id, token);
    }

    fn unregister_active(&self, run_id: u64) {
        self.active_cancellations.lock().unwrap().remove(&run_id);
    }

    fn cancel_active(&self, generation: Option<u64>) {
        let active = self.active_cancellations.lock().unwrap();
        match generation {
            Some(id) => active.get(&id).into_iter().for_each(|token| token.cancel()),
            None => active.values().for_each(|token| token.cancel()),
        }
    }

    fn send(&self, command: WorkerCommand) {
        match &command {
            WorkerCommand::Run(_) => self.cancel_active(None),
            WorkerCommand::Cancel { generation, .. } => self.cancel_active(*generation),
            _ => {}
        }
        let mut state = self.state.lock().unwrap();
        match command {
            WorkerCommand::Run(new_req) => {
                state.settlement.newer_generation();
                let new_id = new_req.run_id;
                // A Run subsumes any immediately-preceding cancel-whatever:
                // the run itself replaces active work, so the bare cancel is
                // redundant (preserves the single-slot overwrite behavior).
                while matches!(
                    state.queue.back(),
                    Some(WorkerCommand::Cancel {
                        generation: None,
                        ..
                    })
                ) {
                    state.queue.pop_back();
                }
                if let Some(pos) = state
                    .queue
                    .iter()
                    .rposition(|command| matches!(command, WorkerCommand::Run(_)))
                {
                    let old_id = match &state.queue[pos] {
                        WorkerCommand::Run(req) => req.run_id,
                        _ => unreachable!("rposition matched a Run"),
                    };
                    state.queue[pos] = WorkerCommand::Run(new_req);
                    self.events.emit(Event::Cancelled {
                        run_id: old_id,
                        superseded_by: Some(new_id),
                    });
                } else {
                    state.queue.push_back(WorkerCommand::Run(new_req));
                }
            }
            WorkerCommand::Cancel {
                generation: Some(id),
                reply,
            } => {
                if let Some(pos) = state.queue.iter().position(
                    |command| matches!(command, WorkerCommand::Run(req) if req.run_id == id),
                ) {
                    // The queued generation never spawns: it is cancelled
                    // before spawn, and the requester is told exactly that.
                    let queued = match state.queue.remove(pos) {
                        Some(WorkerCommand::Run(req)) => req,
                        _ => unreachable!("position matched a Run"),
                    };
                    if let Some(reply) = reply {
                        let _ = reply.send(CancelResult::Cancelled {
                            disposition: CancelDisposition::Graceful,
                            revision: queued.revision,
                            revision_hash: queued.revision_hash,
                        });
                    }
                    self.events.emit(Event::Cancelled {
                        run_id: id,
                        superseded_by: None,
                    });
                } else {
                    state.queue.push_back(WorkerCommand::Cancel {
                        generation: Some(id),
                        reply,
                    });
                }
            }
            WorkerCommand::Cancel {
                generation: None,
                reply,
            } => {
                // cancel-whatever supersedes any queued Run (the original
                // single-slot overwrite behavior), then reaches the consumer
                // to cancel active work.
                while let Some(pos) = state
                    .queue
                    .iter()
                    .position(|command| matches!(command, WorkerCommand::Run(_)))
                {
                    let old_id = match &state.queue[pos] {
                        WorkerCommand::Run(req) => req.run_id,
                        _ => unreachable!("position matched a Run"),
                    };
                    state.queue.remove(pos);
                    self.events.emit(Event::Cancelled {
                        run_id: old_id,
                        superseded_by: None,
                    });
                }
                state.queue.push_back(WorkerCommand::Cancel {
                    generation: None,
                    reply,
                });
            }
            WorkerCommand::ReconcileServices { .. } | WorkerCommand::StartServices { .. } => {
                // Direct command: never coalesced with runs or cancels.
                state.queue.push_back(command);
            }
        }
        self.ready.notify_one();
    }

    fn try_recv(&self) -> Option<WorkerCommand> {
        self.state.lock().unwrap().queue.pop_front()
    }

    fn receive_until_deadline(&self, maximum_wait: Duration) -> SchedulerWake {
        let started = Instant::now();
        let budget_deadline = started + maximum_wait;
        let mut state = self.state.lock().unwrap();
        loop {
            if let Some(command) = state.queue.pop_front() {
                return SchedulerWake::Command(command);
            }
            if state.closed {
                return SchedulerWake::Closed;
            }
            if let Some(deadline) = state.settlement.deadline() {
                let now = Instant::now();
                if now >= deadline {
                    if let Some(claim) = state.settlement.claim_due(now) {
                        return SchedulerWake::SettlementDue(claim.0, claim.1);
                    }
                }
                if now >= budget_deadline {
                    return SchedulerWake::Timeout;
                }
                let wait = deadline
                    .saturating_duration_since(now)
                    .min(budget_deadline.saturating_duration_since(now));
                let (next, timeout) = self.ready.wait_timeout(state, wait).unwrap();
                state = next;
                if timeout.timed_out() {
                    if let Some(claim) = state.settlement.claim_due(Instant::now()) {
                        return SchedulerWake::SettlementDue(claim.0, claim.1);
                    }
                    return SchedulerWake::Timeout;
                }
            } else {
                let now = Instant::now();
                if now >= budget_deadline {
                    return SchedulerWake::Timeout;
                }
                let (next, timeout) = self
                    .ready
                    .wait_timeout(state, budget_deadline.saturating_duration_since(now))
                    .unwrap();
                state = next;
                if timeout.timed_out() {
                    return SchedulerWake::Timeout;
                }
            }
        }
    }

    fn receive(&self) -> Option<WorkerCommand> {
        let mut state = self.state.lock().unwrap();
        while state.queue.is_empty() && !state.closed {
            state = self.ready.wait(state).unwrap();
        }
        state.queue.pop_front()
    }

    fn close(&self) {
        let mut state = self.state.lock().unwrap();
        state.settlement.shutdown();
        state.closed = true;
        self.ready.notify_all();
    }
}

pub struct Worker {
    scheduler: Option<Arc<Scheduler>>,
    next_run_id: AtomicU64,
    root: PathBuf,
    /// Task concurrency bound; part of the execution signature (TASK-0054).
    /// Interior-mutable (TASK-0090 AC7): a reload swaps the shared bound so
    /// newly planned generations use the committed revision's concurrency.
    concurrency: Arc<std::sync::atomic::AtomicUsize>,
    /// Fail-fast policy; part of the execution signature (TASK-0054).
    fail_fast: bool,
    /// Frozen recovery policy, applied to future generations.
    recovery_policy: std::sync::Mutex<crate::config::RecoveryPolicy>,
    recovery_timeout: std::sync::Mutex<Duration>,
    /// Run-level terminal hooks (TASK-0040), applied to target runs.
    /// Interior-mutable (TASK-0092): a reload swaps the shared hooks at the
    /// commit boundary so post-commit generations run the committed hooks
    /// while active runs keep the request they started with.
    hooks: std::sync::Mutex<crate::config::GenerationHooks>,
    /// Immutable config revision all plans prepared through this worker are
    /// frozen under (TASK-0089). Captured before plan creation; a reload
    /// (TASK-0090) swaps it at the commit boundary. Interior mutability so
    /// the reload transaction can swap without rebuilding the worker.
    revision: std::sync::Mutex<Option<crate::config_revision::ConfigRevision>>,

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
        let concurrency = std::thread::available_parallelism()
            .map(|parallelism| parallelism.get())
            .unwrap_or(1);
        Self::with_root_and_concurrency(verbose, fail_fast, root, concurrency, on_event)
    }

    /// Creates a worker with an explicit task-concurrency bound.
    pub fn with_root_and_concurrency<F>(
        verbose: bool,
        fail_fast: bool,
        root: PathBuf,
        concurrency: usize,
        on_event: F,
    ) -> Self
    where
        F: Fn(Event) + Send + Sync + 'static,
    {
        Self::with_root_and_concurrency_and_outputs(
            verbose,
            fail_fast,
            root,
            concurrency,
            on_event,
            None,
        )
    }

    /// Like [`Worker::with_root_and_concurrency`], additionally feeding a
    /// retained-output registry (TASK-0045): each task's stdout/stderr is
    /// captured bounded and recorded per generation.
    pub fn with_root_and_concurrency_and_outputs<F>(
        verbose: bool,
        fail_fast: bool,
        root: PathBuf,
        concurrency: usize,
        on_event: F,
        outputs: Option<Arc<OutputRegistry>>,
    ) -> Self
    where
        F: Fn(Event) + Send + Sync + 'static,
    {
        Self::with_root_and_concurrency_and_outputs_and_approval(
            verbose,
            fail_fast,
            root,
            concurrency,
            on_event,
            outputs,
            Arc::new(crate::approval::TtyRecoveryApproval),
        )
    }

    /// Worker constructor with an explicit approval adapter owned by the
    /// composition root.
    pub fn with_root_and_concurrency_and_outputs_and_approval<F>(
        verbose: bool,
        fail_fast: bool,
        root: PathBuf,
        concurrency: usize,
        on_event: F,
        outputs: Option<Arc<OutputRegistry>>,
        approval: Arc<dyn crate::executor::RecoveryApproval>,
    ) -> Self
    where
        F: Fn(Event) + Send + Sync + 'static,
    {
        let events: Arc<dyn EventSink> = Arc::new(move |event: Event| {
            on_event(event);
        });
        let scheduler = Arc::new(Scheduler::new(Arc::clone(&events)));
        let consumer_scheduler = Arc::clone(&scheduler);
        // TASK-0090 AC7: one shared bound handle drives both the executor's
        // stage planning and the worker's scheduling/signature reads, so a
        // config reload swaps concurrency for newly planned generations
        // without rebuilding the worker or resizing a running group.
        let concurrency_handle = Arc::new(std::sync::atomic::AtomicUsize::new(concurrency));
        let executor = Executor::with_outputs(
            Arc::new(SystemProcessRunner),
            Arc::new(SystemClock),
            concurrency,
            events,
            fail_fast,
            verbose,
            outputs,
        )
        .expect("worker concurrency must be positive")
        .with_recovery_approval(approval)
        .with_concurrency_handle(Arc::clone(&concurrency_handle));

        let consumer = std::thread::spawn(move || {
            let mut active: Option<Run> = None;
            let mut pending: Option<RunRequest> = None;

            loop {
                if active.is_none() {
                    // Promote the newest superseding run, or block on the next
                    // command when idle.
                    if let Some(req) = pending.take() {
                        active = Some(
                            executor.start(
                                RunMetadata::correlated(
                                    req.run_id,
                                    req.trigger.clone(),
                                    req.batch,
                                    req.predecessor,
                                    req.changed.clone(),
                                )
                                .with_duration_profile(
                                    req.target.clone(),
                                    req.execution_signature.clone(),
                                )
                                .with_effective_concurrency(req.effective_concurrency)
                                .with_concurrency_source(req.concurrency_source)
                                .with_hooks(req.hooks.clone())
                                .with_recovery_policy(req.recovery_policy)
                                .with_recovery_timeout(req.recovery_timeout)
                                .with_revision(
                                    req.revision.unwrap_or(0),
                                    req.revision_hash.clone().unwrap_or_default(),
                                ),
                                req.plan,
                            ),
                        );
                        if let Some(run) = active.as_ref() {
                            consumer_scheduler
                                .register_active(run.run_id(), run.cancellation_token());
                        }
                        continue;
                    }

                    match consumer_scheduler.receive() {
                        Some(WorkerCommand::Run(req)) => {
                            active = Some(
                                executor.start(
                                    RunMetadata::correlated(
                                        req.run_id,
                                        req.trigger.clone(),
                                        req.batch,
                                        req.predecessor,
                                        req.changed.clone(),
                                    )
                                    .with_duration_profile(
                                        req.target.clone(),
                                        req.execution_signature.clone(),
                                    )
                                    .with_effective_concurrency(req.effective_concurrency)
                                    .with_concurrency_source(req.concurrency_source)
                                    .with_hooks(req.hooks.clone())
                                    .with_recovery_policy(req.recovery_policy)
                                    .with_recovery_timeout(req.recovery_timeout)
                                    .with_revision(
                                        req.revision.unwrap_or(0),
                                        req.revision_hash.clone().unwrap_or_default(),
                                    ),
                                    req.plan,
                                ),
                            );
                            if let Some(run) = active.as_ref() {
                                consumer_scheduler
                                    .register_active(run.run_id(), run.cancellation_token());
                            }
                        }
                        Some(WorkerCommand::Cancel { generation, reply }) => {
                            // No active run: an exact cancel is a no-op unless
                            // a matching queued Run was already handled by
                            // `send`. reply is only present for exact cancels.
                            if generation.is_some() {
                                if let Some(reply) = reply {
                                    let _ = reply.send(CancelResult::Noop);
                                }
                            }
                        }
                        Some(WorkerCommand::ReconcileServices {
                            stop_names: _,
                            reply,
                        }) => {
                            // No active generation: nothing to stop; every
                            // desired service still needs starting.
                            let _ = reply.send(vec![]);
                        }
                        Some(WorkerCommand::StartServices {
                            run_id,
                            plan,
                            revision,
                        }) => {
                            // No active generation: start the service plan as
                            // its own generation (services keep it alive).
                            active = Some(
                                executor.start(
                                    RunMetadata::correlated(
                                        run_id,
                                        "reload:services".to_owned(),
                                        None,
                                        None,
                                        vec![],
                                    )
                                    .with_revision(
                                        revision.as_ref().map(|r| r.number).unwrap_or(0),
                                        revision
                                            .as_ref()
                                            .map(|r| r.hash.clone())
                                            .unwrap_or_default(),
                                    ),
                                    plan,
                                ),
                            );
                            if let Some(run) = active.as_ref() {
                                consumer_scheduler
                                    .register_active(run.run_id(), run.cancellation_token());
                            }
                        }
                        None => break,
                    }
                    continue;
                }

                let step = executor.advance(active.as_mut().expect("active run"));
                match step {
                    Step::Running => match consumer_scheduler.try_recv() {
                        Some(WorkerCommand::Run(req)) => {
                            let mut replaced = active.take().expect("active run");
                            let replaced_id = replaced.run_id();
                            consumer_scheduler.unregister_active(replaced_id);
                            executor.cancel(&mut replaced, Some(req.run_id));
                            let mut superseding = req;
                            superseding.predecessor = Some(replaced_id);
                            pending = Some(superseding);
                            // Burst drain (TASK-0083/0090): newer Runs already
                            // queued behind this one supersede it in the
                            // pending slot before promotion, so a burst
                            // schedules only the newest generation — never a
                            // cascade of one-run-per-drain starts. Cancels
                            // seen here are answered inline (never dropped):
                            // the replaced run is no longer active, and a
                            // cancel of a queued pending run drops it.
                            loop {
                                match consumer_scheduler.try_recv() {
                                    Some(WorkerCommand::Run(later)) => {
                                        pending = Some(later);
                                    }
                                    Some(WorkerCommand::Cancel {
                                        generation: Some(id),
                                        reply,
                                    }) => {
                                        let cancelled_pending =
                                            pending.as_ref().is_some_and(|req| req.run_id == id);
                                        let pending_revision = pending
                                            .as_ref()
                                            .filter(|req| req.run_id == id)
                                            .map(|req| (req.revision, req.revision_hash.clone()));
                                        if cancelled_pending {
                                            pending = None;
                                        }
                                        if let Some(reply) = reply {
                                            let _ = reply.send(if cancelled_pending {
                                                CancelResult::Cancelled {
                                                    disposition:
                                                        crate::executor::CancelDisposition::Graceful,
                                                    revision: pending_revision
                                                        .as_ref()
                                                        .and_then(|(r, _)| *r),
                                                    revision_hash: pending_revision
                                                        .as_ref()
                                                        .and_then(|(_, h)| h.clone()),
                                                }
                                            } else {
                                                CancelResult::Noop
                                            });
                                        }
                                    }
                                    _ => break,
                                }
                            }
                        }
                        Some(WorkerCommand::ReconcileServices { stop_names, reply }) => {
                            // TASK-0090 AC6: stop the named changed/removed
                            // services owned by the active generation; the
                            // reply names the services still running (the
                            // reload starts new/changed ones under the new
                            // revision). No generation replacement happens, so
                            // active finite work and unchanged services are
                            // untouched (contract §4).
                            if let Some(active) = active.as_mut() {
                                let stop: Vec<&str> =
                                    stop_names.iter().map(String::as_str).collect();
                                let still = executor.reconcile_services(active, &stop);
                                let _ = reply.send(still);
                            } else {
                                let _ = reply.send(vec![]);
                            }
                        }
                        Some(WorkerCommand::StartServices {
                            run_id,
                            plan,
                            revision,
                        }) => {
                            // Append the service-only plan to the ACTIVE
                            // generation: new/changed services start under the
                            // committed revision while active finite work and
                            // unchanged services stay owned (contract §4).
                            if let Some(active) = active.as_mut() {
                                executor.append_plan(active, plan);
                            } else {
                                active = Some(
                                    executor.start(
                                        RunMetadata::correlated(
                                            run_id,
                                            "reload:services".to_owned(),
                                            None,
                                            None,
                                            vec![],
                                        )
                                        .with_revision(
                                            revision.as_ref().map(|r| r.number).unwrap_or(0),
                                            revision
                                                .as_ref()
                                                .map(|r| r.hash.clone())
                                                .unwrap_or_default(),
                                        ),
                                        plan,
                                    ),
                                );
                            }
                        }
                        Some(WorkerCommand::Cancel { generation, reply }) => match generation {
                            Some(id) => {
                                if active.as_ref().is_some_and(|run| run.run_id() == id) {
                                    let mut cancelled = active.take().expect("active run");
                                    consumer_scheduler.unregister_active(id);
                                    let disposition = executor.cancel(&mut cancelled, None);
                                    let revision = cancelled.revision();
                                    if let Some(reply) = reply {
                                        let _ = reply.send(CancelResult::Cancelled {
                                            disposition,
                                            revision: revision.as_ref().map(|r| r.number),
                                            revision_hash: revision
                                                .as_ref()
                                                .map(|r| r.hash.clone()),
                                        });
                                    }
                                } else if let Some(reply) = reply {
                                    let _ = reply.send(CancelResult::Noop);
                                }
                            }
                            None => {
                                if let Some(mut cancelled) = active.take() {
                                    consumer_scheduler.unregister_active(cancelled.run_id());
                                    executor.cancel(&mut cancelled, None);
                                }
                            }
                        },
                        None => std::thread::sleep(Duration::from_millis(200)),
                    },
                    Step::Finished => {
                        let completed_run = active.take().expect("active run");
                        consumer_scheduler.unregister_active(completed_run.run_id());
                        let completed = executor.finish(completed_run);
                        if let Some(spec) = completed.pending_settled_hook.clone() {
                            consumer_scheduler.register_settlement(spec, Instant::now());
                        }
                        stdout::present_results(
                            completed.results,
                            completed.elapsed,
                            Some(&completed.outcome),
                            &completed.tasks,
                        );
                    }
                }
            }

            stdout::info("Consumer thread finished.");
        });

        Worker {
            scheduler: Some(scheduler),
            next_run_id: AtomicU64::new(0),
            root,
            concurrency: concurrency_handle,
            fail_fast,
            recovery_policy: std::sync::Mutex::new(crate::config::RecoveryPolicy::Prompt),
            recovery_timeout: std::sync::Mutex::new(Duration::from_secs(60)),
            hooks: std::sync::Mutex::new(crate::config::GenerationHooks::default()),
            revision: std::sync::Mutex::new(None),
            consumer: Some(consumer),
        }
    }

    pub fn cancel_running_tasks(&self) -> Result<(), String> {
        if let Some(scheduler) = self.scheduler.as_ref() {
            scheduler.send(WorkerCommand::Cancel {
                generation: None,
                reply: None,
            });
        }

        Ok(())
    }

    /// Task concurrency bound the worker executes with; part of the
    /// execution signature (TASK-0054/0055).
    pub fn concurrency(&self) -> usize {
        self.concurrency.load(std::sync::atomic::Ordering::SeqCst)
    }

    /// Swaps the task concurrency bound at the reload commit boundary
    /// (TASK-0090 AC7): plans prepared after this call carry the new bound;
    /// a currently running group is never resized inconsistently.
    pub fn set_concurrency(&self, concurrency: usize) {
        assert!(concurrency > 0, "worker concurrency must be positive");
        self.concurrency
            .store(concurrency, std::sync::atomic::Ordering::SeqCst);
    }

    /// Fail-fast policy the worker executes with; part of the execution
    /// signature (TASK-0054/0055).
    pub fn fail_fast(&self) -> bool {
        self.fail_fast
    }

    /// Attaches the recovery policy applied to future generations.
    pub fn with_recovery_policy(self, policy: crate::config::RecoveryPolicy) -> Self {
        *self.recovery_policy.lock().unwrap() = policy;
        self
    }

    pub fn set_recovery_policy(&self, policy: crate::config::RecoveryPolicy) {
        *self.recovery_policy.lock().unwrap() = policy;
    }

    pub fn with_recovery_timeout(self, timeout: Duration) -> Self {
        *self.recovery_timeout.lock().unwrap() = timeout;
        self
    }

    pub fn set_recovery_timeout(&self, timeout: Duration) {
        *self.recovery_timeout.lock().unwrap() = timeout;
    }

    /// Attaches run-level terminal hooks (TASK-0040) applied to target runs.
    pub fn with_hooks(self, hooks: crate::config::GenerationHooks) -> Self {
        *self.hooks.lock().unwrap() = hooks;
        self
    }

    /// Swaps the run-level terminal hooks at the reload commit boundary
    /// (TASK-0092): plans prepared after this call carry the committed
    /// revision's hooks; active runs keep the hooks they started under.
    pub fn set_hooks(&self, hooks: crate::config::GenerationHooks) {
        *self.hooks.lock().unwrap() = hooks;
    }

    /// Binds the immutable config revision all plans prepared through this
    /// worker are frozen under (TASK-0089, CONFIG-RELOAD-CONTRACT §4).
    pub fn with_revision(self, revision: crate::config_revision::ConfigRevision) -> Self {
        *self.revision.lock().unwrap() = Some(revision);
        self
    }

    /// Swaps the frozen config revision at the reload commit boundary
    /// (TASK-0090). Plans prepared after this call carry the new revision;
    /// active runs keep the revision they started under.
    pub fn set_revision(&self, revision: crate::config_revision::ConfigRevision) {
        *self.revision.lock().unwrap() = Some(revision);
    }

    /// Cancels an exact generation through the worker command stream
    /// (TASK-0046): a compare-and-act on generation identity. Returns whether
    /// the generation matched (active or queued) and how it terminated, or a
    /// no-op when it was already terminal or unknown. Bounded by the shutdown
    /// grace plus a margin; the consumer always replies.
    pub fn cancel_generation(&self, generation: u64) -> Result<CancelResult, String> {
        let Some(scheduler) = self.scheduler.as_ref() else {
            return Err("worker scheduler is unavailable".to_string());
        };
        let (reply, receipt) = std::sync::mpsc::channel();
        scheduler.send(WorkerCommand::Cancel {
            generation: Some(generation),
            reply: Some(reply),
        });
        let (_, grace) = crate::process_owner::shutdown_policy();
        let bound = grace + Duration::from_secs(5);
        receipt
            .recv_timeout(bound)
            .map_err(|_| "cancellation acknowledgement timed out".to_string())
    }

    pub fn schedule(&self, rules: Vec<Rules>, filepath: &str) -> Result<u64, String> {
        self.schedule_plan(RunPlan::from_rules(rules), filepath, None)
    }

    /// Schedules a plan frozen under the given config revision (TASK-0091,
    /// AC7): the caller read the plan and revision under one shared lock, so
    /// the generation freezes exactly that revision. `None` falls back to the
    /// worker's bound revision (legacy/test paths).
    pub fn schedule_plan(
        &self,
        plan: RunPlan,
        filepath: &str,
        revision: Option<crate::config_revision::ConfigRevision>,
    ) -> Result<u64, String> {
        self.schedule_plan_with_trigger(plan, filepath, Some(filepath), revision)
    }

    #[cfg(test)]
    pub(crate) fn schedule_with_trigger(
        &self,
        rules: Vec<Rules>,
        trigger: &str,
        filepath: Option<&str>,
    ) -> Result<u64, String> {
        self.schedule_plan_with_trigger(RunPlan::from_rules(rules), trigger, filepath, None)
    }

    pub(crate) fn schedule_plan_with_trigger(
        &self,
        plan: RunPlan,
        trigger: &str,
        filepath: Option<&str>,
        revision: Option<crate::config_revision::ConfigRevision>,
    ) -> Result<u64, String> {
        self.schedule_plan_correlated(plan, trigger, filepath, None, vec![], revision)
    }

    /// Schedules an exact configured target run with its stable execution
    /// signature (TASK-0054). The signature is computed from the resolved
    /// and expanded plan, so cwd/env/topology changes invalidate history
    /// without parsing the trigger string. Filesystem/init/emit runs go
    /// through [`Worker::schedule_plan_correlated`] and carry no signature,
    /// so they never contaminate target history.
    ///
    /// `sequential` (TASK-0073) requests effective concurrency one for this
    /// exact generation; the signature uses the effective concurrency so
    /// sequential duration history cannot contaminate parallel estimates.
    pub(crate) fn schedule_target(
        &self,
        plan: RunPlan,
        target: &str,
        sequential: bool,
        revision: Option<crate::config_revision::ConfigRevision>,
    ) -> Result<u64, String> {
        let effective = if sequential { 1 } else { self.concurrency() };
        // The trigger label stays `control:<target>` (compatibility surface);
        // profile identity is carried structurally via `target` + signature,
        // never parsed from the trigger string.
        let request = self.prepare_request(
            plan,
            &format!("control:{}", target),
            None,
            None,
            vec![],
            revision,
        )?;
        let request = RunRequest {
            target: Some(target.to_owned()),
            execution_signature: Some(request.plan.execution_signature(effective, self.fail_fast)),
            effective_concurrency: Some(effective),
            concurrency_source: sequential.then_some("control"),
            hooks: self.hooks.lock().unwrap().clone(),
            ..request
        };
        self.dispatch(request)
    }

    /// Schedules a run with its batch correlation (contract §1): the debounce
    /// batch identity and complete changed-path set ride on the generation
    /// from scheduling through start. The predecessor relation is filled by
    /// the consumer when this run supersedes an active one.
    pub(crate) fn schedule_plan_correlated(
        &self,
        plan: RunPlan,
        trigger: &str,
        filepath: Option<&str>,
        batch: Option<u64>,
        changed: Vec<String>,
        revision: Option<crate::config_revision::ConfigRevision>,
    ) -> Result<u64, String> {
        let request = self.prepare_request(plan, trigger, filepath, batch, changed, revision)?;
        self.dispatch(request)
    }

    /// Resolves and expands a plan against the workspace root, emitting the
    /// same verbose diagnostics as every other scheduling path. The frozen
    /// config revision is the caller-provided one when present (read under
    /// the same shared lock as the plan — TASK-0091, AC7); otherwise the
    /// worker's bound revision (legacy/test paths).
    fn prepare_request(
        &self,
        plan: RunPlan,
        trigger: &str,
        filepath: Option<&str>,
        batch: Option<u64>,
        changed: Vec<String>,
        caller_revision: Option<crate::config_revision::ConfigRevision>,
    ) -> Result<RunRequest, String> {
        let plan = plan.resolve_context(&self.root)?;
        let (plan, unknown_variables) = plan.expand(&TemplateOptions {
            filepath: filepath.map(str::to_string),
            // TASK-0031: the complete normalized changed-path set rides the
            // generation; expose it as {{paths}} for batch-aware commands.
            paths: changed.clone(),
            current_dir: format!("{}", self.root.display()),
        });
        for variable in unknown_variables {
            stdout::warn(&format!("Unknown template variable '{}'.", variable));
        }
        let run_id = self.next_run_id.fetch_add(1, Ordering::Relaxed) + 1;
        let revision = caller_revision.or_else(|| self.revision.lock().unwrap().clone());
        Ok(RunRequest {
            run_id,
            plan,
            trigger: trigger.to_string(),
            batch,
            changed,
            predecessor: None,
            target: None,
            execution_signature: None,
            effective_concurrency: None,
            concurrency_source: None,
            // TASK-0092: the WATCH path must carry the worker's current
            // hooks (startup hooks at first; the reload commit swaps them via
            // `set_hooks`), exactly like the control path — never the empty
            // default.
            hooks: self.hooks.lock().unwrap().clone(),
            recovery_policy: *self.recovery_policy.lock().unwrap(),
            recovery_timeout: *self.recovery_timeout.lock().unwrap(),
            revision: revision.as_ref().map(|r| r.number),
            revision_hash: revision.as_ref().map(|r| r.hash.clone()),
        })
    }

    /// Reconciles managed services after a reload commit (TASK-0090 AC6):
    /// gracefully stops the named services owned by the active generation
    /// (changed/removed by the new revision) and returns the names still
    /// running. The caller starts the new/changed services under the new
    /// revision; unchanged services remain owned. Synchronous, bounded by the
    /// shutdown grace plus a margin.
    pub fn reconcile_services(&self, stop_names: Vec<String>) -> Result<Vec<String>, String> {
        let Some(scheduler) = self.scheduler.as_ref() else {
            return Err("worker scheduler is unavailable".to_string());
        };
        let (reply, receipt) = std::sync::mpsc::channel();
        scheduler.send(WorkerCommand::ReconcileServices { stop_names, reply });
        let (_, grace) = crate::process_owner::shutdown_policy();
        let bound = grace + Duration::from_secs(5);
        receipt
            .recv_timeout(bound)
            .map_err(|_| "service reconciliation timed out".to_string())
    }

    /// Starts new/changed managed services under the current revision without
    /// replacing the active generation (TASK-0090 AC6). The service-only plan
    /// is appended to the active run, or becomes its own generation when the
    /// worker is idle. Active finite work and unchanged services stay owned.
    pub fn start_services(&self, plan: RunPlan) -> Result<u64, String> {
        let Some(scheduler) = self.scheduler.as_ref() else {
            return Err("worker scheduler is unavailable".to_string());
        };
        let run_id = self.next_run_id.fetch_add(1, Ordering::Relaxed) + 1;
        let revision = self.revision.lock().unwrap().clone();
        scheduler.send(WorkerCommand::StartServices {
            run_id,
            plan,
            revision,
        });
        Ok(run_id)
    }

    /// Sends a prepared run request through the scheduler.
    fn dispatch(&self, request: RunRequest) -> Result<u64, String> {
        if let Some(scheduler) = self.scheduler.as_ref() {
            let run_id = request.run_id;
            scheduler.send(WorkerCommand::Run(request));
            return Ok(run_id);
        }

        Err("worker scheduler is unavailable".to_string())
    }
}

impl Drop for Worker {
    fn drop(&mut self) {
        if let Some(scheduler) = self.scheduler.as_ref() {
            scheduler.send(WorkerCommand::Cancel {
                generation: None,
                reply: None,
            });
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
    use std::path::{Path, PathBuf};
    use std::sync::mpsc::{channel, Receiver};
    use std::time::Instant;

    struct BlockingApproval;

    struct ApprovingApproval;

    impl crate::executor::RecoveryApproval for ApprovingApproval {
        fn approve(
            &self,
            _requests: &[crate::executor::RecoveryRequest],
            cancellation: &crate::executor::CancellationToken,
            _timeout: Duration,
        ) -> crate::executor::ApprovalDecision {
            if cancellation.is_cancelled() {
                crate::executor::ApprovalDecision::Cancelled
            } else {
                crate::executor::ApprovalDecision::Approved
            }
        }
    }

    impl crate::executor::RecoveryApproval for BlockingApproval {
        fn approve(
            &self,
            _requests: &[crate::executor::RecoveryRequest],
            cancellation: &crate::executor::CancellationToken,
            _timeout: Duration,
        ) -> crate::executor::ApprovalDecision {
            while !cancellation.is_cancelled() {
                std::thread::sleep(Duration::from_millis(5));
            }
            crate::executor::ApprovalDecision::Cancelled
        }
    }

    fn output_file(name: &str) -> PathBuf {
        std::env::temp_dir().join(format!("funzzy-worker-{}-{}", std::process::id(), name))
    }

    fn write_file_rule(path: &Path) -> Rules {
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

    struct GatedApproval {
        requests: Arc<Mutex<Vec<crate::executor::RecoveryRequest>>>,
        released: Arc<std::sync::atomic::AtomicBool>,
    }

    impl crate::executor::RecoveryApproval for GatedApproval {
        fn approve(
            &self,
            requests: &[crate::executor::RecoveryRequest],
            cancellation: &crate::executor::CancellationToken,
            _timeout: Duration,
        ) -> crate::executor::ApprovalDecision {
            self.requests.lock().unwrap().extend_from_slice(requests);
            while !self.released.load(std::sync::atomic::Ordering::SeqCst)
                && !cancellation.is_cancelled()
            {
                std::thread::sleep(Duration::from_millis(5));
            }
            if cancellation.is_cancelled() {
                crate::executor::ApprovalDecision::Cancelled
            } else {
                crate::executor::ApprovalDecision::Approved
            }
        }
    }

    #[test]
    fn parallel_recovery_waits_for_original_siblings_and_serializes_passes() {
        let root = output_file("parallel-recovery");
        let _ = std::fs::remove_dir_all(&root);
        std::fs::create_dir_all(&root).unwrap();
        let first = root.join("first");
        let second = root.join("second");
        let first_running = root.join("first.running");
        let second_running = root.join("second.running");
        let recovery_one = root.join("recovery-one.running");
        let recovery_two = root.join("recovery-two.running");
        let overlap = root.join("overlap");
        let run = |marker: &Path, running: &Path| {
            format!(
                "if test -f '{}'; then exit 0; else touch '{}'; sleep 0.1; rm -f '{}'; exit 1; fi",
                marker.display(),
                running.display(),
                running.display()
            )
        };
        let recovery = |marker: &Path, running: &Path, other: &Path| {
            format!(
                "if test -f '{}' || test -f '{}'; then touch '{}'; exit 1; fi; touch '{}'; sleep 0.05; rm -f '{}'; touch '{}'",
                other.display(),
                recovery_two.display(),
                overlap.display(),
                running.display(),
                running.display(),
                marker.display()
            )
        };
        let (tx, rx) = channel();
        let worker = Worker::with_root_and_concurrency_and_outputs_and_approval(
            false,
            false,
            std::env::current_dir().unwrap(),
            2,
            move |event| tx.send(event).unwrap(),
            None,
            Arc::new(ApprovingApproval),
        );
        let rules = vec![
            Rules::new(
                "first".to_owned(),
                vec![run(&first, &first_running)],
                vec![],
                vec![],
                true,
            )
            .with_parallel("checks".to_owned())
            .with_recovery(vec![recovery(&first, &recovery_one, &second_running)]),
            Rules::new(
                "second".to_owned(),
                vec![run(&second, &second_running)],
                vec![],
                vec![],
                true,
            )
            .with_parallel("checks".to_owned())
            .with_recovery(vec![recovery(&second, &recovery_two, &recovery_one)]),
        ];
        let run_id = worker
            .schedule_with_trigger(rules, "parallel", None)
            .unwrap();
        expect_event(
            &rx,
            "parallel recovery finished",
            |event| matches!(event, WorkerEvent::Finished { run_id: id, .. } if *id == run_id),
        );
        assert!(first.exists() && second.exists());
        assert!(!overlap.exists(), "recovery phases must not overlap");
        let _ = std::fs::remove_dir_all(root);
    }

    #[test]
    fn recovery_request_keeps_frozen_revision_and_commands_after_reload() {
        let requests = Arc::new(Mutex::new(Vec::new()));
        let released = Arc::new(std::sync::atomic::AtomicBool::new(false));
        let (tx, rx) = channel();
        let worker = Worker::with_root_and_concurrency_and_outputs_and_approval(
            false,
            true,
            std::env::current_dir().unwrap(),
            1,
            move |event| tx.send(event).unwrap(),
            None,
            Arc::new(GatedApproval {
                requests: Arc::clone(&requests),
                released: Arc::clone(&released),
            }),
        );
        let old = crate::config_revision::ConfigRevision {
            number: 11,
            hash: "old-revision".to_owned(),
        };
        let plan = RunPlan::from_rules(vec![Rules::new(
            "recoverable".to_owned(),
            vec!["false".to_owned()],
            vec![],
            vec![],
            true,
        )
        .with_recovery(vec!["echo old-recovery".to_owned()])]);
        worker
            .schedule_plan_with_trigger(plan, "old-trigger", None, Some(old.clone()))
            .unwrap();
        let deadline = Instant::now() + Duration::from_secs(5);
        while requests.lock().unwrap().is_empty() && Instant::now() < deadline {
            std::thread::sleep(Duration::from_millis(10));
        }
        assert_eq!(requests.lock().unwrap().len(), 1);

        worker.set_revision(crate::config_revision::ConfigRevision {
            number: 12,
            hash: "new-revision".to_owned(),
        });
        worker.set_recovery_policy(crate::config::RecoveryPolicy::Skip);
        released.store(true, std::sync::atomic::Ordering::SeqCst);
        let request = requests.lock().unwrap()[0].clone();
        assert_eq!(request.revision, Some(old.number));
        assert_eq!(request.commands, ["echo old-recovery"]);
        expect_event(
            &rx,
            "finished",
            |event| matches!(event, WorkerEvent::Finished { run_id: id, .. } if *id == request.generation),
        );
    }

    #[test]
    fn cancellation_during_recovery_reaps_child_and_skips_terminal_hooks() {
        let marker = output_file("cancel-recovery-child.hook");
        let (tx, rx) = channel();
        let worker = Worker::with_root_and_concurrency_and_outputs_and_approval(
            false,
            true,
            std::env::current_dir().unwrap(),
            1,
            move |event| tx.send(event).unwrap(),
            None,
            Arc::new(ApprovingApproval),
        )
        .with_hooks(crate::config::GenerationHooks {
            // failure_settle defaults to None for legacy callers
            success: None,
            failure: Some(format!("touch '{}'", marker.display())),
            failure_settle: None,
        });
        let run_id = worker
            .schedule_with_trigger(
                vec![Rules::new(
                    "recoverable".to_owned(),
                    vec!["false".to_owned()],
                    vec![],
                    vec![],
                    true,
                )
                .with_recovery(vec!["sleep 30".to_owned()])],
                "test",
                None,
            )
            .unwrap();
        expect_event(&rx, "recovery started", |event| {
            matches!(
                event,
                WorkerEvent::RecoveryPhase {
                    run_id: id,
                    phase,
                    ..
                } if *id == run_id && phase == "recovery_started"
            )
        });
        assert!(matches!(
            worker.cancel_generation(run_id).unwrap(),
            CancelResult::Cancelled { .. }
        ));
        expect_event(
            &rx,
            "cancelled",
            |event| matches!(event, WorkerEvent::Cancelled { run_id: id, .. } if *id == run_id),
        );
        assert!(!marker.exists(), "cancelled generations must not run hooks");
        let _ = std::fs::remove_file(marker);
    }

    #[test]
    fn cancellation_during_verification_reaps_child_and_skips_terminal_hooks() {
        let marker = output_file("cancel-verification.marker");
        let hook = output_file("cancel-verification.hook");
        let run = format!(
            "if test -f '{}'; then sleep 30; else exit 1; fi",
            marker.display()
        );
        let (tx, rx) = channel();
        let worker = Worker::with_root_and_concurrency_and_outputs_and_approval(
            false,
            true,
            std::env::current_dir().unwrap(),
            1,
            move |event| tx.send(event).unwrap(),
            None,
            Arc::new(ApprovingApproval),
        )
        .with_hooks(crate::config::GenerationHooks {
            // failure_settle defaults to None for legacy callers
            success: Some(format!("touch '{}'", hook.display())),
            failure: Some(format!("touch '{}'", hook.display())),
            failure_settle: None,
        });
        let run_id = worker
            .schedule_with_trigger(
                vec![
                    Rules::new("recoverable".to_owned(), vec![run], vec![], vec![], true)
                        .with_recovery(vec![format!("touch '{}'", marker.display())]),
                ],
                "test",
                None,
            )
            .unwrap();
        expect_event(&rx, "verification started", |event| {
            matches!(
                event,
                WorkerEvent::RecoveryPhase {
                    run_id: id,
                    phase,
                    ..
                } if *id == run_id && phase == "verification_started"
            )
        });
        assert!(matches!(
            worker.cancel_generation(run_id).unwrap(),
            CancelResult::Cancelled { .. }
        ));
        expect_event(
            &rx,
            "cancelled",
            |event| matches!(event, WorkerEvent::Cancelled { run_id: id, .. } if *id == run_id),
        );
        assert!(!hook.exists(), "cancelled generations must not run hooks");
        let _ = std::fs::remove_file(marker);
        let _ = std::fs::remove_file(hook);
    }

    #[test]
    fn cancellation_during_recovery_approval_reaps_and_skips_terminal_hooks() {
        let root = output_file("cancel-recovery-approval");
        let marker = root.with_extension("hook");
        let (tx, rx) = channel();
        let worker = Worker::with_root_and_concurrency_and_outputs_and_approval(
            false,
            true,
            std::env::current_dir().unwrap(),
            1,
            move |event| tx.send(event).unwrap(),
            None,
            Arc::new(BlockingApproval),
        )
        .with_hooks(crate::config::GenerationHooks {
            // failure_settle defaults to None for legacy callers
            success: None,
            failure: Some(format!("touch '{}'", marker.display())),
            failure_settle: None,
        });
        let run_id = worker
            .schedule_with_trigger(
                vec![Rules::new(
                    "recoverable".to_owned(),
                    vec!["false".to_owned()],
                    vec![],
                    vec![],
                    true,
                )
                .with_recovery(vec!["true".to_owned()])],
                "test",
                None,
            )
            .unwrap();
        expect_event(&rx, "approval requested", |event| {
            matches!(
                event,
                WorkerEvent::RecoveryPhase {
                    run_id: id,
                    phase,
                    ..
                } if *id == run_id && phase == "approval_requested"
            )
        });

        let result = worker.cancel_generation(run_id).unwrap();
        assert!(matches!(result, CancelResult::Cancelled { .. }));
        expect_event(&rx, "cancelled", |event| {
            matches!(
                event,
                WorkerEvent::Cancelled { run_id: id, .. } if *id == run_id
            )
        });
        assert!(!marker.exists(), "cancelled generations must not run hooks");
        let _ = std::fs::remove_file(marker);
        let _ = std::fs::remove_file(root);
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

    #[test]
    fn finished_settled_failure_registers_exact_pending_spec() {
        let (worker, rx) = worker_with_events(false, false);
        worker.set_hooks(crate::config::GenerationHooks {
            success: None,
            failure: Some("notify".into()),
            failure_settle: Some(Duration::from_secs(9)),
        });
        worker.set_revision(crate::config_revision::ConfigRevision {
            number: 12,
            hash: "rev12".into(),
        });
        let run_id = worker
            .schedule(vec![rule(vec!["false"])], "fail.rs")
            .unwrap();
        let _ = expect_event(
            &rx,
            "finished",
            |e| matches!(e, WorkerEvent::Finished { run_id: id, .. } if *id == run_id),
        );
        let scheduler = worker.scheduler.as_ref().unwrap().clone();
        let state = scheduler.state.lock().unwrap();
        match &state.settlement {
            SettlementState::Pending { spec, .. } => {
                assert_eq!(spec.run_id, run_id);
                assert_eq!(spec.command, "notify");
                assert_eq!(spec.settle, Duration::from_secs(9));
                assert_eq!(spec.revision, 12);
                assert_eq!(spec.revision_hash, "rev12");
            }
            _ => panic!("expected pending settled hook"),
        }
        drop(state);
        drop(worker);
    }

    #[test]
    fn schedule_cancels_claimed_settlement() {
        let (worker, rx) = worker_with_events(false, false);
        let scheduler = worker.scheduler.as_ref().unwrap().clone();
        let now = Instant::now();
        let spec = crate::executor::PendingSettledHook {
            run_id: 7,
            command: "x".into(),
            settle: Duration::from_secs(1),
            revision: 1,
            revision_hash: "h".into(),
        };
        scheduler.register_settlement(spec, now);
        let (_, token) = scheduler
            .claim_settlement(now + Duration::from_secs(1))
            .unwrap();
        let run_id = worker
            .schedule(vec![rule(vec!["true"])], "next.rs")
            .unwrap();
        assert!(token.is_cancelled());
        assert!(matches!(
            scheduler.state.lock().unwrap().settlement,
            SettlementState::Idle
        ));
        let event = expect_event(
            &rx,
            "run started",
            |e| matches!(e, WorkerEvent::Started { run_id: id, .. } if *id == run_id),
        );
        assert!(matches!(event, WorkerEvent::Started { .. }));
        drop(worker);
    }

    #[test]
    fn schedule_cancels_pending_settlement() {
        let (worker, rx) = worker_with_events(false, false);
        let scheduler = worker.scheduler.as_ref().unwrap().clone();
        let now = Instant::now();
        scheduler.register_settlement(
            crate::executor::PendingSettledHook {
                run_id: 6,
                command: "x".into(),
                settle: Duration::from_secs(10),
                revision: 1,
                revision_hash: "h".into(),
            },
            now,
        );
        let run_id = worker
            .schedule(vec![rule(vec!["true"])], "next.rs")
            .unwrap();
        let _ = expect_event(
            &rx,
            "run started",
            |e| matches!(e, WorkerEvent::Started { run_id: id, .. } if *id == run_id),
        );
        assert!(matches!(
            scheduler.state.lock().unwrap().settlement,
            SettlementState::Idle
        ));
        drop(worker);
    }

    #[test]
    fn scheduler_receive_empty_returns_timeout_at_zero_wait() {
        let scheduler = Scheduler::new(Arc::new(|_| {}));
        assert!(matches!(
            scheduler.receive_until_deadline(Duration::ZERO),
            SchedulerWake::Timeout
        ));
    }

    #[test]
    fn scheduler_receive_zero_due_beats_timeout() {
        let scheduler = Scheduler::new(Arc::new(|_| {}));
        scheduler.register_settlement(
            crate::executor::PendingSettledHook {
                run_id: 10,
                command: "x".into(),
                settle: Duration::ZERO,
                revision: 1,
                revision_hash: "h".into(),
            },
            Instant::now(),
        );
        assert!(
            matches!(scheduler.receive_until_deadline(Duration::ZERO), SchedulerWake::SettlementDue(spec, _) if spec.run_id == 10)
        );
    }

    #[test]
    fn scheduler_receive_prioritizes_queued_command_over_future_settlement() {
        let scheduler = Scheduler::new(Arc::new(|_| {}));
        let now = Instant::now();
        scheduler.register_settlement(
            crate::executor::PendingSettledHook {
                run_id: 8,
                command: "x".into(),
                settle: Duration::from_secs(60),
                revision: 1,
                revision_hash: "h".into(),
            },
            now,
        );
        let (tx, _rx) = std::sync::mpsc::channel();
        scheduler.send(WorkerCommand::ReconcileServices {
            stop_names: vec![],
            reply: tx,
        });
        assert!(matches!(
            scheduler.receive_until_deadline(Duration::from_secs(60)),
            SchedulerWake::Command(WorkerCommand::ReconcileServices { .. })
        ));
        assert!(matches!(
            scheduler.state.lock().unwrap().settlement,
            SettlementState::Pending { .. }
        ));
    }

    #[test]
    fn scheduler_receive_returns_closed_after_close() {
        let scheduler = Scheduler::new(Arc::new(|_| {}));
        scheduler.close();
        assert!(matches!(
            scheduler.receive_until_deadline(Duration::from_secs(60)),
            SchedulerWake::Closed
        ));
    }

    #[test]
    fn scheduler_receive_claims_due_settlement_once() {
        let scheduler = Scheduler::new(Arc::new(|_| {}));
        let now = Instant::now();
        let spec = crate::executor::PendingSettledHook {
            run_id: 8,
            command: "x".into(),
            settle: Duration::ZERO,
            revision: 1,
            revision_hash: "h".into(),
        };
        scheduler.register_settlement(spec.clone(), now);
        let wake = scheduler.receive_until_deadline(Duration::from_secs(1));
        match wake {
            SchedulerWake::SettlementDue(claimed, token) => {
                assert_eq!(claimed, spec);
                assert!(!token.is_cancelled());
            }
            _ => panic!("expected settlement due"),
        }
        assert!(scheduler.claim_settlement(now).is_none());
    }

    #[test]
    fn scheduler_registration_keeps_newest_generation() {
        let (worker, _) = worker_with_events(false, false);
        let scheduler = worker.scheduler.as_ref().unwrap().clone();
        let now = Instant::now();
        scheduler.register_settlement(
            crate::executor::PendingSettledHook {
                run_id: 7,
                command: "a".into(),
                settle: Duration::from_secs(5),
                revision: 1,
                revision_hash: "a".into(),
            },
            now,
        );
        scheduler.register_settlement(
            crate::executor::PendingSettledHook {
                run_id: 8,
                command: "b".into(),
                settle: Duration::from_secs(10),
                revision: 2,
                revision_hash: "b".into(),
            },
            now,
        );
        assert!(scheduler
            .claim_settlement(now + Duration::from_secs(5))
            .is_none());
        let (claimed, _) = scheduler
            .claim_settlement(now + Duration::from_secs(10))
            .unwrap();
        assert_eq!(claimed.run_id, 8);
        drop(worker);
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
    fn schedule_with_explicit_revision_freezes_that_revision_on_the_generation() {
        // TASK-0091, AC7: the caller reads plan + revision under one shared
        // lock and passes the revision explicitly; the scheduled generation
        // must freeze exactly that revision (a concurrent commit cannot
        // re-freeze it with a later revision).
        let (worker, rx) = worker_with_events(false, false);
        let explicit = crate::config_revision::ConfigRevision {
            number: 7,
            hash: "hash-7".to_owned(),
        };
        worker
            .schedule_plan(
                RunPlan::from_rules(vec![rule(vec!["echo ok"])]),
                "a.txt",
                Some(explicit),
            )
            .expect("schedules");

        let event = expect_event(&rx, "started with frozen revision", |event| {
            matches!(event, WorkerEvent::Started { .. })
        });
        if let WorkerEvent::Started {
            revision,
            revision_hash,
            ..
        } = event
        {
            assert_eq!(revision, Some(7));
            assert_eq!(revision_hash.as_deref(), Some("hash-7"));
        } else {
            panic!("expected Started");
        }
        // Drain to terminal so the worker's consumer thread stops cleanly.
        expect_event(&rx, "terminal", |event| {
            matches!(
                event,
                WorkerEvent::Finished { .. } | WorkerEvent::Cancelled { .. }
            )
        });
    }

    #[test]
    fn schedule_without_revision_falls_back_to_the_worker_bound_revision() {
        let (worker, rx) = worker_with_events(false, false);
        let worker = worker.with_revision(crate::config_revision::ConfigRevision {
            number: 3,
            hash: "hash-3".to_owned(),
        });
        worker
            .schedule_plan(
                RunPlan::from_rules(vec![rule(vec!["echo ok"])]),
                "a.txt",
                None,
            )
            .expect("schedules");

        let event = expect_event(&rx, "started with worker revision", |event| {
            matches!(event, WorkerEvent::Started { .. })
        });
        if let WorkerEvent::Started {
            revision,
            revision_hash,
            ..
        } = event
        {
            assert_eq!(revision, Some(3));
            assert_eq!(revision_hash.as_deref(), Some("hash-3"));
        } else {
            panic!("expected Started");
        }
        // Drain to terminal so the worker's consumer thread stops cleanly.
        expect_event(&rx, "terminal", |event| {
            matches!(
                event,
                WorkerEvent::Finished { .. } | WorkerEvent::Cancelled { .. }
            )
        });
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
            events
                .iter()
                .any(|e| matches!(e, WorkerEvent::Cancelled { .. })),
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
            matches!(e, WorkerEvent::Cancelled { .. })
        });
        drop(worker);
        assert!(
            rx.recv_timeout(Duration::from_millis(300)).is_err(),
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
    fn replacement_records_predecessor_and_superseded_relations() {
        let (worker, rx) = worker_with_events(false, false);
        let first = worker
            .schedule(vec![rule(vec!["sleep 5"])], "a.txt")
            .unwrap();
        expect_event(
            &rx,
            "first run to start",
            |e| matches!(e, WorkerEvent::Started { run_id, .. } if *run_id == first),
        );
        expect_event(&rx, "worker tick", |e| {
            matches!(e, WorkerEvent::Tick { .. })
        });

        let second = worker
            .schedule(vec![rule(vec!["echo ok"])], "b.txt")
            .unwrap();
        let events = collect_until_finished(&rx);
        drop(worker);

        let cancelled = events
            .iter()
            .find_map(|e| match e {
                WorkerEvent::Cancelled {
                    run_id,
                    superseded_by,
                } => Some((*run_id, *superseded_by)),
                _ => None,
            })
            .expect("replacement cancellation must be recorded");
        assert_eq!(
            cancelled,
            (first, Some(second)),
            "the superseded generation names its successor"
        );

        let predecessor = events
            .iter()
            .find_map(|e| match e {
                WorkerEvent::Started {
                    run_id,
                    predecessor,
                    ..
                } if *run_id == second => Some(*predecessor),
                _ => None,
            })
            .expect("superseding generation must start");
        assert_eq!(
            predecessor,
            Some(first),
            "the superseding generation names its predecessor"
        );
    }

    #[test]
    fn discarded_queued_generation_reports_superseded_terminal() {
        let (worker, rx) = worker_with_events(false, false);
        let first = worker
            .schedule(vec![rule(vec!["sleep 5"])], "a.txt")
            .unwrap();
        expect_event(
            &rx,
            "first run to start",
            |e| matches!(e, WorkerEvent::Started { run_id, .. } if *run_id == first),
        );
        expect_event(&rx, "worker tick", |e| {
            matches!(e, WorkerEvent::Tick { .. })
        });

        // Two rapid schedules: the middle one is discarded from the queue
        // before spawn and must still reach a terminal superseded outcome.
        let middle = worker
            .schedule(vec![rule(vec!["echo mid"])], "b.txt")
            .unwrap();
        let last = worker
            .schedule(vec![rule(vec!["echo last"])], "c.txt")
            .unwrap();

        let events = collect_until_finished(&rx);
        drop(worker);

        let discarded = events.iter().find_map(|e| match e {
            WorkerEvent::Cancelled {
                run_id,
                superseded_by,
            } => Some((*run_id, *superseded_by)),
            _ => None,
        });
        assert!(
            matches!(discarded, Some((run_id, superseded_by)) if run_id == middle && superseded_by == Some(last)),
            "the discarded queued generation must reach superseded terminal: {events:?}"
        );
    }

    #[test]
    fn cancel_generation_cancels_the_active_run() {
        let (worker, rx) = worker_with_events(false, false);
        let run_id = worker
            .schedule(vec![rule(vec!["sleep 5"])], "a.txt")
            .unwrap();
        expect_event(
            &rx,
            "run to start",
            |e| matches!(e, WorkerEvent::Started { run_id: id, .. } if *id == run_id),
        );

        let result = worker.cancel_generation(run_id).unwrap();
        assert!(
            matches!(
                result,
                CancelResult::Cancelled {
                    disposition: CancelDisposition::Graceful,
                    ..
                }
            ),
            "expected graceful cancellation, got {result:?}"
        );

        expect_event(&rx, "run to be cancelled", |e| {
            matches!(
                e,
                WorkerEvent::Cancelled {
                    run_id: id,
                    superseded_by: None
                } if *id == run_id
            )
        });
        drop(worker);
    }

    #[test]
    fn cancel_generation_noops_after_terminal() {
        let (worker, rx) = worker_with_events(false, false);
        let run_id = worker
            .schedule(vec![rule(vec!["echo ok"])], "a.txt")
            .unwrap();
        collect_until_finished(&rx);

        assert_eq!(
            worker.cancel_generation(run_id).unwrap(),
            CancelResult::Noop
        );
        drop(worker);
    }

    #[test]
    fn cancel_generation_noops_for_unknown_generation() {
        let (worker, _rx) = worker_with_events(false, false);
        assert_eq!(worker.cancel_generation(99).unwrap(), CancelResult::Noop);
        drop(worker);
    }

    #[test]
    fn cancel_generation_cancels_a_queued_run_before_spawn() {
        let (worker, rx) = worker_with_events(false, false);
        let first = worker
            .schedule(vec![rule(vec!["sleep 5"])], "a.txt")
            .unwrap();
        expect_event(
            &rx,
            "first run to start",
            |e| matches!(e, WorkerEvent::Started { run_id: id, .. } if *id == first),
        );
        expect_event(&rx, "worker tick", |e| {
            matches!(e, WorkerEvent::Tick { .. })
        });

        let second = worker
            .schedule(vec![rule(vec!["echo ok"])], "b.txt")
            .unwrap();
        let result = worker.cancel_generation(second).unwrap();
        assert!(
            matches!(result, CancelResult::Cancelled { .. }),
            "queued generation must be cancelled, got {result:?}"
        );
        drop(worker);
    }

    #[test]
    fn stale_cancel_does_not_affect_a_newer_generation() {
        let (worker, rx) = worker_with_events(false, false);
        let first = worker
            .schedule(vec![rule(vec!["sleep 5"])], "a.txt")
            .unwrap();
        expect_event(
            &rx,
            "first run to start",
            |e| matches!(e, WorkerEvent::Started { run_id: id, .. } if *id == first),
        );
        expect_event(&rx, "worker tick", |e| {
            matches!(e, WorkerEvent::Tick { .. })
        });

        // Replace first with second, then send a stale cancel for first.
        let second = worker
            .schedule(vec![rule(vec!["echo ok"])], "b.txt")
            .unwrap();
        collect_until_finished(&rx);

        // first is now superseded; a cancel for it must be a no-op and must
        // not touch second (already passed).
        assert_eq!(worker.cancel_generation(first).unwrap(), CancelResult::Noop);
        drop(worker);
        let _ = second;
    }

    #[test]
    fn generation_ids_are_never_reused_after_terminal() {
        let (worker, rx) = worker_with_events(false, false);

        let first = worker
            .schedule(vec![rule(vec!["echo ok"])], "a.txt")
            .unwrap();
        collect_until_finished(&rx);

        let second = worker
            .schedule(vec![rule(vec!["echo ok"])], "b.txt")
            .unwrap();
        collect_until_finished(&rx);
        drop(worker);

        assert!(
            second > first,
            "generation ids must be strictly increasing across terminal outcomes"
        );
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

    #[test]
    fn target_runs_record_duration_history_through_the_worker_path() {
        use crate::duration_recorder::DurationRecorder;
        use crate::duration_store::DurationStore;
        use crate::plan::RunPlan;

        let temp = std::env::temp_dir().join(format!(
            "funzzy-worker-target-history-{}",
            std::process::id()
        ));
        let _ = std::fs::remove_dir_all(&temp);
        std::fs::create_dir_all(&temp).unwrap();

        // Control-run path: schedule_target attaches target + signature
        // structurally; the recorder observes the worker's events and records
        // the terminal wall duration against the profile.
        let store = DurationStore::new(temp.join("run-durations-v1.json"));
        let recorder = Arc::new(DurationRecorder::new(store));
        let recorder_state = Arc::clone(&recorder);
        let worker =
            Worker::with_root_and_concurrency(false, false, temp.clone(), 1, move |event| {
                recorder_state.observe(&event)
            });
        let plan = RunPlan::from_rules(vec![rule(vec!["echo ok"])]);
        // The worker hashes the resolved+expanded plan; the test must match.
        let resolved = plan.resolve_context(&temp).expect("resolve");
        let signature = resolved.execution_signature(1, false);

        worker
            .schedule_target(plan, "build", false, None)
            .expect("target schedules");
        // Wait until the run reaches terminal (worker polls at 200ms max).
        let deadline = std::time::Instant::now() + std::time::Duration::from_secs(10);
        while recorder.success_samples(&signature) == 0 && std::time::Instant::now() < deadline {
            std::thread::sleep(std::time::Duration::from_millis(50));
        }
        drop(worker);

        assert_eq!(
            recorder.success_samples(&signature),
            1,
            "control run must record one success sample"
        );
        assert_eq!(recorder.in_flight(), 0, "association removed at terminal");
        let estimate = recorder.estimate(&signature, None).expect("estimate");
        assert_eq!(estimate.samples, 1);
        let _ = std::fs::remove_dir_all(&temp);
    }
}

#[cfg(test)]
mod manual_frozen_reload_tests {
    use super::*;
    use crate::executor::Event as WorkerEvent;
    use crate::plan::RunPlan;
    use crate::rules::{Rules, TriggerMode};
    use std::sync::mpsc::{channel, Receiver};
    use std::time::Duration;

    /// MANUAL-TRIGGER-CONTRACT §7/§11 (Kely's real-reload requirement): a
    /// manual target scheduled under revision N keeps N after a reload
    /// commits trigger-only revision N+1, while a post-reload explicit
    /// manual run binds N+1.
    #[test]
    fn manual_generation_keeps_frozen_revision_across_reload_commit() {
        let manual = Rules::new(
            "await-remote".to_owned(),
            vec!["sleep 30".to_owned()],
            vec![],
            vec![],
            false,
        )
        .with_trigger(Some(TriggerMode::Manual));
        let (worker, rx) = worker_with_events(false, false);

        // Generation under revision N.
        worker.set_revision(crate::config_revision::ConfigRevision {
            number: 4,
            hash: "hash-4".to_owned(),
        });
        let revision_n = crate::config_revision::ConfigRevision {
            number: 4,
            hash: "hash-4".to_owned(),
        };
        let plan = RunPlan::from_rules(vec![manual]);
        let run_n = worker
            .schedule_target(plan, "await-remote", false, Some(revision_n))
            .expect("manual target schedules under revision N");

        let started = expect_event(&rx, "started under N", |event| {
            matches!(event, WorkerEvent::Started { .. })
        });
        let frozen = if let WorkerEvent::Started {
            run_id,
            revision,
            revision_hash,
            ..
        } = started
        {
            assert_eq!(run_id, run_n);
            (revision, revision_hash)
        } else {
            panic!("expected Started");
        };
        assert_eq!(frozen.0, Some(4), "generation freezes revision N");
        assert_eq!(frozen.1.as_deref(), Some("hash-4"));

        // Reload commits trigger-only revision N+1 while the manual run is
        // live (sleep 30 keeps it running).
        worker.set_revision(crate::config_revision::ConfigRevision {
            number: 5,
            hash: "hash-5".to_owned(),
        });

        // The LIVE generation still reports N: cancel it and the terminal
        // event/snapshot attribution retains the frozen revision.
        let cancelled = worker.cancel_generation(run_n).expect("cancel frozen run");
        let crate::workers::CancelResult::Cancelled {
            revision,
            revision_hash,
            ..
        } = cancelled
        else {
            panic!("frozen generation must have been active (matched cancel)");
        };
        assert_eq!(
            revision,
            Some(4),
            "terminal attribution keeps frozen revision N"
        );
        assert_eq!(revision_hash.as_deref(), Some("hash-4"));
        let terminal = expect_event(&rx, "terminal retains frozen revision", |event| {
            matches!(
                event,
                WorkerEvent::Cancelled { .. } | WorkerEvent::Finished { .. }
            )
        });
        if let WorkerEvent::Cancelled { run_id, .. } = terminal {
            assert_eq!(run_id, run_n, "the frozen generation itself terminated");
        }

        // Post-reload explicit manual run binds N+1.
        let manual_next = Rules::new(
            "await-remote".to_owned(),
            vec!["echo done".to_owned()],
            vec![],
            vec![],
            false,
        )
        .with_trigger(Some(TriggerMode::Manual));
        let revision_n1 = crate::config_revision::ConfigRevision {
            number: 5,
            hash: "hash-5".to_owned(),
        };
        let run_n1 = worker
            .schedule_target(
                RunPlan::from_rules(vec![manual_next]),
                "await-remote",
                false,
                Some(revision_n1),
            )
            .expect("post-reload manual target schedules");
        assert_ne!(run_n, run_n1);
        let started_next = expect_event(&rx, "started under N+1", |event| {
            matches!(event, WorkerEvent::Started { .. })
        });
        if let WorkerEvent::Started {
            run_id,
            revision,
            revision_hash,
            ..
        } = started_next
        {
            assert_eq!(run_id, run_n1);
            assert_eq!(revision, Some(5), "post-reload run binds N+1");
            assert_eq!(revision_hash.as_deref(), Some("hash-5"));
        } else {
            panic!("expected Started");
        }
        expect_event(&rx, "drain terminal", |event| {
            matches!(event, WorkerEvent::Finished { .. })
        });
    }

    fn worker_with_events(verbose: bool, fail_fast: bool) -> (Worker, Receiver<WorkerEvent>) {
        let (tx, rx) = channel();
        (
            Worker::new(verbose, fail_fast, move |event| {
                let _ = tx.send(event);
            }),
            rx,
        )
    }

    fn expect_event(
        rx: &Receiver<WorkerEvent>,
        description: &str,
        mut matches: impl FnMut(&WorkerEvent) -> bool,
    ) -> WorkerEvent {
        loop {
            let event = rx
                .recv_timeout(Duration::from_secs(10))
                .unwrap_or_else(|_| panic!("timed out waiting for: {description}"));
            if matches(&event) {
                return event;
            }
        }
    }
    #[test]
    fn settlement_transitions_are_deterministic() {
        let now = Instant::now();
        let spec = crate::executor::PendingSettledHook {
            run_id: 7,
            command: "x".into(),
            settle: Duration::from_secs(5),
            revision: 2,
            revision_hash: "h".into(),
        };
        let mut state = SettlementState::new();
        state.register(spec.clone(), now);
        assert!(state.claim_due(now + Duration::from_secs(4)).is_none());
        let (claimed, token) = state.claim_due(now + Duration::from_secs(5)).unwrap();
        assert_eq!(claimed, spec.clone());
        assert!(!token.is_cancelled());
        assert!(state.claim_due(now + Duration::from_secs(6)).is_none());
        state.register(spec.clone(), now);
        let (_, old_token) = state.claim_due(now + Duration::from_secs(5)).unwrap();
        state.newer_generation();
        assert!(old_token.is_cancelled());
        assert!(matches!(state, SettlementState::Idle));
        let newer = crate::executor::PendingSettledHook {
            run_id: 8,
            settle: Duration::from_secs(10),
            ..spec
        };
        state.register(newer.clone(), now);
        assert!(state.claim_due(now + Duration::from_secs(5)).is_none());
        let (claimed_new, new_token) = state.claim_due(now + Duration::from_secs(10)).unwrap();
        assert_eq!(claimed_new.run_id, 8);
        state.shutdown();
        assert!(new_token.is_cancelled());
        assert!(matches!(state, SettlementState::Idle));
    }
    #[test]
    fn scheduler_close_cancels_claimed_settlement() {
        let scheduler = Scheduler::new(Arc::new(|_| {}));
        let now = Instant::now();
        let spec = crate::executor::PendingSettledHook {
            run_id: 9,
            command: "x".into(),
            settle: Duration::from_secs(1),
            revision: 1,
            revision_hash: "h".into(),
        };
        {
            let mut state = scheduler.state.lock().unwrap();
            state.settlement.register(spec, now);
        }
        let (_, token) = scheduler
            .claim_settlement(now + Duration::from_secs(1))
            .unwrap();
        scheduler.close();
        assert!(token.is_cancelled());
        assert!(matches!(
            scheduler.state.lock().unwrap().settlement,
            SettlementState::Idle
        ));
    }
}
