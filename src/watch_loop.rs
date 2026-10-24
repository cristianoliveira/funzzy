//! Shared watch orchestration.
//!
//! One application flow owns filesystem readiness and event-to-run
//! conversion. Blocking and cancellable behaviors are injected as
//! [`RunStrategy`] implementations; init and change triggers share one
//! preparation path. CLI commands stay thin: build a strategy and call
//! [`watch_loop`].

use crate::awaiting::AwaitCoordinator;
use crate::config_revision::ConfigRevision;
use crate::control::{
    ControlApi, ControlRunError, ControlServer, ControlTarget, EmitOutcome, ScheduledRun,
    TargetEstimateProvider,
};
use crate::diagnostics;
use crate::duration_recorder::DurationRecorder;
use crate::errors::FzzError;
use crate::executor::RunMetadata;
use crate::identity::{Batch, BatchId};
use crate::output::OutputRegistry;
use crate::path_context;
use crate::plan::RunPlan;
use crate::snapshot::SnapshotBroker;
use crate::stdout;
use crate::watcher::{self, FileEvent};
use crate::watcher_state::{WatcherInstance, WatcherState};
use crate::watches::Watches;
use crate::workers;
use crate::workflow::WorkflowRunner;
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex};
use std::time::Duration;

/// What the watch loop does once filesystem watches are registered.
pub enum InitAction {
    /// Init-selected plan should run now.
    Run(RunPlan),
    /// Nothing to run on init; wait for file changes.
    Wait,
}

/// One preparation path for init triggers: run when rules exist AND the
/// `run_on_init` flag is enabled, otherwise wait for changes.
pub fn init_action(plan: Option<RunPlan>, run_on_init: bool) -> InitAction {
    match plan {
        Some(plan) if run_on_init => InitAction::Run(plan),
        _ => InitAction::Wait,
    }
}

/// Injected executor strategy: owns how selected rules are executed.
pub trait RunStrategy {
    /// Updates generation hooks for future blocking runs. Asynchronous
    /// strategies freeze their hooks at schedule time and may leave this as
    /// a no-op.
    fn update_generation_hooks(&self, _hooks: crate::config::GenerationHooks) {}

    /// Called once after every filesystem watch is registered, before any
    /// init work, so auxiliary surfaces (e.g. the control socket) publish
    /// only when readiness is truthful. Defaults to no-op.
    fn on_ready(&self) {}

    /// Executes plan selected for init. Returns the scheduled generation when
    /// the strategy schedules asynchronous work (restart policy); blocking
    /// strategies run in-process and return `None`. `revision` is the frozen
    /// config revision read under the same shared lock as the plan
    /// (TASK-0091, AC7): the scheduled generation must freeze exactly that
    /// revision, never a later commit's.
    fn run_init(&self, plan: RunPlan, revision: Option<ConfigRevision>) -> Option<u64>;

    /// The busy-run policy this strategy implements: `restart` replaces the
    /// active run with newer work; `wait` runs in-process and blocks.
    fn policy(&self) -> &'static str;

    /// Executes plan selected for a file change. The batch carries the
    /// debounce identity and complete changed-path set (contract §1); the
    /// trigger path is the deterministic first match. Returns the scheduled
    /// generation for non-blocking strategies, `None` for in-process runs.
    /// `revision` is the frozen config revision read under the same shared
    /// lock as the plan (TASK-0091, AC7).
    fn run_change(
        &self,
        plan: RunPlan,
        filepath: &str,
        batch: &Batch,
        revision: Option<ConfigRevision>,
    ) -> Option<u64>;

    /// Called with each normalized batch before routing (default no-op), so
    /// the pending-debounce observation reflects open windows.
    fn on_batch(&self, _batch: &Batch) {}

    /// Called after the batch finished routing (scheduled or explicit no-op).
    fn on_batch_complete(&self) {}

    /// Schedules the complete configured workflow for an explicit keyboard
    /// trigger. Returns a generation for asynchronous strategies.
    fn run_manual(&self, plan: RunPlan, revision: Option<ConfigRevision>) -> Option<u64>;

    /// Retries only the failed and timed-out finite jobs of the latest
    /// terminal run (Ctrl-R, KEYBOARD-RETRY-CONTRACT). Implementations own
    /// their selection source and present every rejection; they never fall
    /// back to a full run. Returns the scheduled generation for asynchronous
    /// strategies, `None` when the retry ran in-process or was rejected.
    fn retry_latest(&self, watches: &std::sync::Arc<std::sync::Mutex<Watches>>) -> Option<u64> {
        let _ = watches;
        None
    }

    /// Whether the strategy currently has an in-flight generation.
    fn is_running(&self) -> bool {
        false
    }

    /// Whether a generation returned by `run_manual` has reached a terminal
    /// state. Pending scheduler requests are not complete even before the
    /// worker publishes their first `started` event.
    fn is_generation_complete(&self, _generation: u64) -> bool {
        true
    }
}

/// Runs the watch loop: registers filesystem watches, publishes readiness,
/// converts init and change events into rule selections, and delegates
/// execution to the injected strategy.
///
/// `watches` is a shared handle (TASK-0090): the config-reload transaction
/// swaps the effective configuration under the lock at the commit boundary,
/// and each batch routes under exactly one committed revision. `swap_rx`
/// carries live root swaps to the backend; None for legacy/blocking callers.
/// Content-change gate (TASK-0114): notify backends can re-deliver a path
/// in later debounce windows even though nothing wrote it again (observed
/// on Linux whenever two watcher instances cover one tree, e.g. the jobs
/// watcher plus the config-reload watcher). Routing follows actual
/// modification, in the same spirit as the reload watcher's config
/// baselines: a path routes only when its mtime changed since it last
/// routed (first sighting routes; a deletion routes once). Chatter without
/// a real write can never schedule work on any backend or platform.
struct ModificationGate {
    last_seen: std::collections::HashMap<String, Option<std::time::SystemTime>>,
}

impl ModificationGate {
    fn new() -> Self {
        ModificationGate {
            last_seen: std::collections::HashMap::new(),
        }
    }

    /// Baselines every existing file at the configured baseline paths
    /// (TASK-0114): a pre-existing file may never route as a "first sighting"
    /// — the §4 directory walk synthesizes pre-existing siblings on Linux (a
    /// file create bumps the parent dir, whose walk re-lists them). Entries
    /// are fill-only, so callers can seed disjoint paths without masking an
    /// already-tracked write. Live reloads deliberately do NOT re-seed: a
    /// file created under a newly added path must route on first sighting.
    fn seed(&mut self, paths: &[String]) {
        for configured_path in paths {
            let configured_path = std::path::Path::new(configured_path);
            if configured_path.is_file() {
                self.record_baseline(configured_path);
                continue;
            }

            let mut descendants = Vec::new();
            watcher::walk_descendants(configured_path, &mut descendants);
            for path in descendants {
                self.record_baseline(&path);
            }
        }
    }

    fn record_baseline(&mut self, path: &std::path::Path) {
        let mtime = std::fs::metadata(path)
            .and_then(|meta| meta.modified())
            .ok();
        if let Some(path) = path.to_str() {
            self.last_seen.entry(path.to_owned()).or_insert(mtime);
        }
    }

    /// Returns the paths that EXIST and whose observed mtime differs from
    /// the previous call, in input order. Deletions never schedule work
    /// (nothing to run); an absent path updates the baseline so a later
    /// recreation routes exactly once.
    fn changed(&mut self, paths: Vec<String>) -> Vec<String> {
        let mut routed = Vec::new();
        for path in paths {
            let current = std::fs::metadata(&path)
                .and_then(|meta| meta.modified())
                .ok();
            let previous = self.last_seen.insert(path.clone(), current);
            match (previous, current) {
                // First sighting of an existing path, or a real modification.
                (_, Some(mtime)) if previous != Some(Some(mtime)) => routed.push(path),
                // Creation after a known deletion.
                (Some(None), Some(_)) => routed.push(path),
                // Same mtime (chatter re-delivery) or absent path: quiet.
                _ => {}
            }
        }
        routed
    }
}

#[allow(clippy::too_many_arguments)]
pub fn watch_loop(
    watches: &std::sync::Arc<std::sync::Mutex<Watches>>,
    run_on_init: bool,
    strategy: &dyn RunStrategy,
    debounce: Duration,
    verbose: bool,
    swap_rx: Option<crate::watcher::RootSwapReceiver>,
    // Optional readiness gate: the main loop waits for this signal before
    // running init, so a config-touching init task can never fire before
    // the reload watcher has registered its roots (TASK-0090).
    reload_ready: Option<std::sync::Arc<std::sync::Mutex<std::sync::mpsc::Receiver<()>>>>,
    shutdown: Option<std::sync::Arc<crate::shutdown::ShutdownCoordinator>>,
) -> Result<(), FzzError> {
    let initial = watches.lock().unwrap().clone();
    let list_of_watched_paths = initial.paths_to_watch().unwrap_or_default();
    let shutdown_flag = shutdown
        .as_ref()
        .map(|coordinator| coordinator.requested_flag());
    let ready_shutdown = shutdown;
    let gate = std::cell::RefCell::new(ModificationGate::new());
    let baseline_paths = initial
        .baseline_paths()
        .into_iter()
        .map(|path| path.display().to_string())
        .collect::<Vec<_>>();
    gate.borrow_mut().seed(&baseline_paths);

    let shortcut_rx = crate::shortcut::start_reader(shutdown_flag.clone());
    let mut trigger_latch = crate::shortcut::TriggerLatch::default();
    let mut trigger_phase = ManualTriggerPhase::Idle;
    let handle_shortcut = move |key: Option<crate::shortcut::KeyDecode>| {
        let Some(key) = key else {
            if let ManualTriggerPhase::Active(generation) = trigger_phase {
                if strategy.is_generation_complete(generation) {
                    trigger_phase = ManualTriggerPhase::Idle;
                    trigger_latch.reset();
                }
            }
            if matches!(trigger_phase, ManualTriggerPhase::Waiting) && !strategy.is_running() {
                run_latched_action(strategy, watches, &mut trigger_latch, &mut trigger_phase);
            }
            return;
        };
        if key == crate::shortcut::KeyDecode::Eof {
            return;
        }
        if !trigger_latch.accept(key) {
            if matches!(
                key,
                crate::shortcut::KeyDecode::Trigger | crate::shortcut::KeyDecode::Retry
            ) {
                stdout::info("Shortcut already latched; waiting for the current run to finish.");
            }
            return;
        }
        if strategy.is_running() {
            trigger_phase = ManualTriggerPhase::Waiting;
            stdout::info("Shortcut latched; waiting for the current run to finish.");
        } else {
            run_latched_action(strategy, watches, &mut trigger_latch, &mut trigger_phase);
        }
    };

    watcher::events_with_shortcut(
        list_of_watched_paths,
        || {
            if let Some(ready) = &reload_ready {
                let _ = ready.lock().unwrap().recv_timeout(Duration::from_secs(30));
            }
            strategy.on_ready();
            if let Some(shutdown) = &ready_shutdown {
                shutdown.mark_ready();
            }

            let initial = watches.lock().unwrap().clone();
            strategy.update_generation_hooks(initial.hooks());
            let init_revision = initial.revision().cloned();
            match init_action(initial.run_on_init_plan(), run_on_init) {
                InitAction::Run(plan) => {
                    stdout::info("Running on init commands.");
                    let commands = plan.commands().len();
                    let generation = strategy.run_init(plan, init_revision);
                    // Blocking strategies emit their own in-process run
                    // record; only non-blocking init schedules are recorded
                    // here with their generation.
                    if verbose && generation.is_some() {
                        diagnostics::debug(&diagnostics::Record {
                            source: Some("init"),
                            decision: Some("scheduled"),
                            generation,
                            policy: Some(strategy.policy()),
                            commands: Some(commands),
                            ..Default::default()
                        });
                    }
                }
                InitAction::Wait => stdout::info("Watching..."),
            }
        },
        |batch_id: u64, events: &[FileEvent]| {
            let batch = Batch::normalized(
                BatchId(batch_id),
                events.iter().map(|event| event.path.clone()).collect(),
            );
            if batch.is_empty() {
                return;
            }
            strategy.on_batch(&batch);
            // Content-change gate (TASK-0114): only paths actually modified
            // since their last routed batch may schedule work; notify's
            // chatter re-delivery is filtered before matching.
            let changed_paths = gate.borrow_mut().changed(batch.changed.clone());
            if changed_paths.is_empty() {
                strategy.on_batch_complete();
                return;
            }
            let batch = Batch::normalized(batch.id, changed_paths);
            // Lock once per batch: the whole routing decision (match/ignore,
            // plan, trigger, frozen revision) reads one committed revision
            // (contract §4). The revision rides the schedule so the generated
            // run freezes exactly the routed revision (TASK-0091, AC7).
            let watches_guard = watches.lock().unwrap();
            strategy.update_generation_hooks(watches_guard.hooks());
            let revision = watches_guard.revision().cloned();
            match watches_guard.watch_plan_batch(&batch.changed) {
                Some((plan, trigger)) => {
                    stdout::clear_screen();
                    if verbose {
                        emit_matched_decisions(&watches_guard, &batch, &trigger);
                    }
                    let generation = strategy.run_change(plan, &trigger, &batch, revision);
                    if verbose {
                        observe_triggers(&watches_guard, &batch, &trigger, generation);
                    }
                }
                None => {
                    if verbose {
                        emit_non_matched_decisions(&watches_guard, &batch);
                    }
                }
            }
            drop(watches_guard);
            strategy.on_batch_complete();
        },
        debounce,
        initial.backend(),
        verbose,
        swap_rx,
        shutdown_flag,
        Some((shortcut_rx, Box::new(handle_shortcut))),
    )
    .map_err(FzzError::GenericError)
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum ManualTriggerPhase {
    Idle,
    Waiting,
    Active(u64),
}

/// Presents one keyboard-retry rejection without ever falling back to a full
/// run. Messages mirror the control-socket retry outcomes so the terminal
/// and socket clients learn the same reasons.
fn present_retry_rejection(error: &crate::control::ControlRetryError) {
    match error {
        crate::control::ControlRetryError::SourceUnavailable { .. } => {
            stdout::warn("Shortcut ignored: no previous run to retry.");
        }
        crate::control::ControlRetryError::SourceNotTerminal { .. } => {
            stdout::warn(
                "Shortcut ignored: latest run has not finished; try again when it is terminal.",
            );
        }
        crate::control::ControlRetryError::StaleRevision { .. } => {
            stdout::warn(
                "Shortcut ignored: config changed after the latest run; trigger a fresh run first.",
            );
        }
        crate::control::ControlRetryError::NoRetryableJobs { .. } => {
            stdout::warn("Shortcut ignored: latest run has no failed or timed-out jobs to retry.");
        }
        crate::control::ControlRetryError::UnsupportedService { name } => {
            stdout::warn(&format!(
                "Shortcut ignored: service job '{}' cannot be retried.",
                name
            ));
        }
        crate::control::ControlRetryError::WatcherBusy => {
            stdout::warn(
                "Shortcut ignored: watcher is busy; press Ctrl-R again after the current run.",
            );
        }
        crate::control::ControlRetryError::Internal(message) => {
            stdout::error(&format!(
                "failed to retry from keyboard shortcut: {message}"
            ));
        }
    }
}

/// Runs the one latched shortcut action: Ctrl-G the full pipeline, Ctrl-R
/// the failed-job retry. The latch carries exactly one action; the first
/// shortcut received while a run is active wins.
fn run_latched_action(
    strategy: &dyn RunStrategy,
    watches: &Arc<Mutex<Watches>>,
    latch: &mut crate::shortcut::TriggerLatch,
    phase: &mut ManualTriggerPhase,
) {
    match latch.latched() {
        Some(crate::shortcut::ShortcutAction::RetryFailed) => {
            trigger_retry_run(strategy, watches, latch, phase)
        }
        _ => trigger_manual_run(strategy, watches, latch, phase),
    }
}

fn trigger_retry_run(
    strategy: &dyn RunStrategy,
    watches: &Arc<Mutex<Watches>>,
    latch: &mut crate::shortcut::TriggerLatch,
    phase: &mut ManualTriggerPhase,
) {
    // Hooks and plan are owned by each strategy under its own locks: the
    // non-block worker freezes committed hooks per schedule, and the blocking
    // strategy reads one committed watches snapshot. The loop only dispatches.
    if let Some(generation) = strategy.retry_latest(watches) {
        *phase = ManualTriggerPhase::Active(generation);
    } else {
        latch.reset();
        *phase = ManualTriggerPhase::Idle;
    }
}

fn trigger_manual_run(
    strategy: &dyn RunStrategy,
    watches: &Arc<Mutex<Watches>>,
    latch: &mut crate::shortcut::TriggerLatch,
    phase: &mut ManualTriggerPhase,
) {
    let (plan, revision, hooks) = {
        let watches = watches.lock().unwrap();
        (
            watches.manual_trigger_plan(),
            watches.revision().cloned(),
            watches.hooks(),
        )
    };
    strategy.update_generation_hooks(hooks);
    let Some(plan) = plan else {
        stdout::warn("Shortcut ignored: no configured jobs to run.");
        latch.reset();
        *phase = ManualTriggerPhase::Idle;
        return;
    };
    stdout::info("Running full pipeline from keyboard shortcut.");
    if let Some(generation) = strategy.run_manual(plan, revision) {
        *phase = ManualTriggerPhase::Active(generation);
    } else {
        latch.reset();
        *phase = ManualTriggerPhase::Idle;
    }
}

/// Emits one `matched` decision record per task responsible for the trigger
/// path, plus `ignored`/`unmatched` decisions for the remaining batch paths
/// that did not win the deterministic first match.
fn emit_matched_decisions(watches: &Watches, batch: &Batch, trigger: &str) {
    let explained = watches.explain(trigger);
    for rule in &explained.matched {
        diagnostics::debug(&diagnostics::Record {
            batch: Some(batch.id.0),
            decision: Some("matched"),
            task: Some(rule.name.clone()),
            change: rule.change_patterns.first().cloned(),
            rule_origin: Some(rule.origin.clone()),
            ..Default::default()
        });
    }
    for path in batch.changed.iter().filter(|path| *path != trigger) {
        emit_path_decision(watches, batch, path);
    }
}

/// Emits `ignored` (with the winning ignore rule and its origin) or
/// `unmatched` decisions for a batch whose paths never matched a task.
fn emit_non_matched_decisions(watches: &Watches, batch: &Batch) {
    for path in &batch.changed {
        emit_path_decision(watches, batch, path);
    }
}

/// One decision per path: `ignored` when an ignore rule won, else
/// `unmatched`. Never executes work.
fn emit_path_decision(watches: &Watches, batch: &Batch, path: &str) {
    let explained = watches.explain(path);
    if !explained.ignored.is_empty() {
        for rule in &explained.ignored {
            diagnostics::debug(&diagnostics::Record {
                batch: Some(batch.id.0),
                decision: Some("ignored"),
                task: Some(rule.name.clone()),
                ignore: rule.ignore_patterns.first().cloned(),
                rule_origin: Some(rule.origin.clone()),
                path: Some(path.to_owned()),
                normalized: Some(watches.normalized_path(path)),
                ..Default::default()
            });
        }
        return;
    }
    if explained.matched.is_empty() {
        diagnostics::debug(&diagnostics::Record {
            batch: Some(batch.id.0),
            decision: Some("unmatched"),
            path: Some(path.to_owned()),
            normalized: Some(watches.normalized_path(path)),
            ..Default::default()
        });
    }
}

/// Feeds the loop heuristic for every matched task and relays its
/// observational output: `observed_after_run` correlation records and the
/// bounded `possible feedback loop` warning. Diagnostics never alter the
/// scheduled generation or task results.
fn observe_triggers(watches: &Watches, batch: &Batch, trigger: &str, generation: Option<u64>) {
    let explained = watches.explain(trigger);
    for rule in &explained.matched {
        let change = rule.change_patterns.first().cloned().unwrap_or_default();
        let Some(observation) = diagnostics::observe(&rule.name, trigger, &change, generation)
        else {
            continue;
        };
        if let Some(warning) = &observation.warning {
            diagnostics::warn_loop(warning);
        }
        if let Some(after) = observation.observed_after_run {
            diagnostics::debug(&diagnostics::Record {
                batch: Some(batch.id.0),
                source: Some("filesystem"),
                kind: Some("any"),
                path: Some(trigger.to_owned()),
                normalized: Some(watches.normalized_path(trigger)),
                observed_after_run: Some(after),
                ..Default::default()
            });
        }
    }
}

/// Blocking executor: expands command templates and runs tasks in-process,
/// honoring fail-fast, then presents the results.
pub struct BlockingStrategy {
    workflow: WorkflowRunner,
    /// Latest terminal in-process run outcome (KEYBOARD-RETRY-CONTRACT):
    /// terminal task snapshots plus the trigger path that produced them, so
    /// Ctrl-R can re-run exactly the failed/timed-out finite jobs with the
    /// original filepath context.
    latest_run: Mutex<Option<LatestBlockingRun>>,
}

#[derive(Clone)]
struct LatestBlockingRun {
    tasks: Vec<crate::task_result::TaskSnapshot>,
    filepath: Option<String>,
}

impl BlockingStrategy {
    pub fn new(root: PathBuf, verbose: bool, fail_fast: bool, concurrency: usize) -> Self {
        Self::with_events(root, verbose, fail_fast, concurrency, None)
    }

    /// Creates the blocking strategy with an optional NDJSON run-event stream
    /// (TASK-0039).
    pub fn with_events(
        root: PathBuf,
        verbose: bool,
        fail_fast: bool,
        concurrency: usize,
        events: Option<Arc<crate::event_stream::EventStream>>,
    ) -> Self {
        Self {
            workflow: WorkflowRunner::with_recorder_and_events(
                root,
                verbose,
                fail_fast,
                concurrency,
                None,
                events,
            ),
            latest_run: Mutex::new(None),
        }
    }

    /// Attaches run-level terminal hooks (TASK-0040).
    pub fn with_hooks(mut self, hooks: crate::config::GenerationHooks) -> Self {
        self.workflow = self.workflow.with_hooks(hooks);
        self
    }

    pub fn with_recovery_policy(mut self, policy: crate::config::RecoveryPolicy) -> Self {
        self.workflow = self.workflow.with_recovery_policy(policy);
        self
    }

    pub fn with_recovery_approval(
        mut self,
        approval: Arc<dyn crate::executor::RecoveryApproval>,
    ) -> Self {
        self.workflow = self.workflow.with_recovery_approval(approval);
        self
    }
}

impl RunStrategy for BlockingStrategy {
    fn update_generation_hooks(&self, hooks: crate::config::GenerationHooks) {
        self.workflow.update_generation_hooks(hooks);
    }

    fn policy(&self) -> &'static str {
        "wait"
    }

    fn run_manual(&self, plan: RunPlan, _revision: Option<ConfigRevision>) -> Option<u64> {
        match self
            .workflow
            .run(plan, RunMetadata::new(0, "keyboard"), None)
        {
            Ok(completed) => {
                self.record_completed_run(&completed, None);
                stdout::present_results(
                    completed.results,
                    completed.elapsed,
                    Some(&completed.outcome),
                    &completed.tasks,
                );
            }
            Err(error) => stdout::error(&error),
        }
        None
    }

    fn run_init(&self, plan: RunPlan, _revision: Option<ConfigRevision>) -> Option<u64> {
        match self.workflow.run(plan, RunMetadata::new(0, "init"), None) {
            Ok(completed) => {
                self.record_completed_run(&completed, None);
                stdout::present_results(
                    completed.results,
                    completed.elapsed,
                    Some(&completed.outcome),
                    &completed.tasks,
                );
            }
            Err(error) => stdout::error(&error),
        }
        None
    }

    fn run_change(
        &self,
        plan: RunPlan,
        filepath: &str,
        batch: &Batch,
        _revision: Option<ConfigRevision>,
    ) -> Option<u64> {
        let metadata =
            RunMetadata::correlated(0, filepath, Some(batch.id.0), None, batch.changed.clone());
        match self.workflow.run(plan, metadata, Some(filepath)) {
            Ok(completed) => {
                self.record_completed_run(&completed, Some(filepath));
                stdout::present_results(
                    completed.results,
                    completed.elapsed,
                    Some(&completed.outcome),
                    &completed.tasks,
                );
            }
            Err(error) => stdout::error(&error),
        }
        None
    }

    fn retry_latest(&self, watches: &Arc<Mutex<Watches>>) -> Option<u64> {
        let latest = self.latest_run.lock().unwrap().clone();
        let Some(latest) = latest else {
            stdout::warn("Shortcut ignored: no previous run to retry.");
            return None;
        };
        let selected = match crate::retry::retryable_task_names(&latest.tasks) {
            Ok(selected) => selected,
            Err(crate::retry::RetrySelectionError::NoRetryableJobs) => {
                stdout::warn(
                    "Shortcut ignored: latest run has no failed or timed-out jobs to retry.",
                );
                return None;
            }
        };

        let (plan, hooks) = {
            let watches = watches.lock().unwrap();
            // One committed snapshot decides the retry: plan selection and
            // terminal hooks must come from the same revision, never split
            // across a reload commit (TASK-0091 semantics).
            let plan = watches.retry_plan(&selected);
            (plan, watches.hooks())
        };
        if plan.is_empty() || plan.task_names() != selected {
            stdout::warn(
                "Shortcut ignored: config changed after the latest run; trigger a fresh run first.",
            );
            return None;
        }
        self.workflow.update_generation_hooks(hooks);
        for task in plan.stages.iter().flat_map(|stage| match stage {
            crate::plan::Stage::Serial(task) => vec![task],
            crate::plan::Stage::Parallel { tasks, .. } => tasks.iter().collect(),
        }) {
            if task.service {
                stdout::warn(&format!(
                    "Shortcut ignored: service job '{}' cannot be retried.",
                    task.name
                ));
                return None;
            }
        }

        stdout::info("Retrying failed jobs from keyboard shortcut.");
        let metadata = RunMetadata::new(0, "retry");
        match self
            .workflow
            .run(plan, metadata, latest.filepath.as_deref())
        {
            Ok(completed) => {
                self.record_completed_run(&completed, latest.filepath.as_deref());
                stdout::present_results(
                    completed.results,
                    completed.elapsed,
                    Some(&completed.outcome),
                    &completed.tasks,
                );
            }
            Err(error) => stdout::error(&error),
        }
        None
    }
}

impl BlockingStrategy {
    fn record_completed_run(
        &self,
        completed: &crate::executor::CompletedRun,
        filepath: Option<&str>,
    ) {
        *self.latest_run.lock().unwrap() = Some(LatestBlockingRun {
            tasks: completed.tasks.clone(),
            filepath: filepath.map(str::to_owned),
        });
    }
}

/// Cancellable executor: schedules runs on the worker, cancelling any active
/// run before replacement work, and publishes the control surface after
/// readiness.
///
/// Target/emit/estimate decisions read the SHARED watch config (TASK-0091,
/// AC6/AC7): the reload transaction swaps it at the commit boundary, so
/// `targets`, `run`, and `emit` after a valid reload reflect the new
/// revision and every decision binds to exactly one revision under one lock.
pub struct NonBlockStrategy {
    worker: Arc<workers::Worker>,
    /// Shared watch config: the same handle the routing loop locks per batch
    /// and the reload transaction swaps at commit. Never a private copy — a
    /// private copy would go stale on reload.
    shared: Arc<std::sync::Mutex<Watches>>,
    control_socket: Option<PathBuf>,
    control_state: Arc<Mutex<WatcherState>>,
    /// Serializes explicit retry validation/scheduling against other generation
    /// scheduling paths without changing native restart behavior.
    schedule_lock: Mutex<()>,
    /// Bounded idempotency cache: retrying the same source returns the
    /// already-scheduled generation instead of scheduling another.
    retry_cache: Mutex<std::collections::BTreeMap<u64, crate::control::RetryRun>>,
    /// Original `{{filepath}}` context keyed by generation. This avoids
    /// inferring provenance from trigger strings, which may be user paths.
    generation_filepaths: Mutex<std::collections::BTreeMap<u64, Option<String>>>,
    coordinator: Option<Arc<AwaitCoordinator>>,
    outputs: Option<Arc<OutputRegistry>>,
    instance: Arc<WatcherInstance>,
    broker: Option<Arc<SnapshotBroker>>,
    control_server: Mutex<Option<ControlServer>>,
    /// Old server parked during a socket-path handoff (AC8); dropped on
    /// retire after the commit boundary.
    pending_old_server: Mutex<Option<ControlServer>>,
    self_arc: Mutex<Option<Arc<NonBlockStrategy>>>,
    /// Optional duration recorder (TASK-0055): wires the estimate provider
    /// into `targets`, capabilities, and correlated snapshots.
    recorder: Option<Arc<DurationRecorder>>,
    /// Optional config lifecycle source (TASK-0091, AC3): wired from the
    /// reload coordinator; the control server serves `config` and await
    /// observations carry the live transition.
    lifecycle: Option<Arc<crate::config_lifecycle::ConfigLifecycle>>,
}

impl NonBlockStrategy {
    /// Creates the strategy inside an `Arc`; the arc lets the control server
    /// call back through the run orchestration contract. The watch config is
    /// wrapped in a fresh shared handle (no reload wiring).
    pub fn new_arc(
        worker: Arc<workers::Worker>,
        watches: Watches,
        control_socket: Option<PathBuf>,
        control_state: Arc<Mutex<WatcherState>>,
        coordinator: Option<Arc<AwaitCoordinator>>,
        outputs: Option<Arc<OutputRegistry>>,
    ) -> Arc<Self> {
        Self::new_arc_with_shared(
            worker,
            Arc::new(std::sync::Mutex::new(watches)),
            control_socket,
            control_state,
            coordinator,
            outputs,
            Arc::new(WatcherInstance::new()),
            None,
            None,
            None,
        )
    }

    /// Creates the strategy with a shared instance and subscription broker
    /// (TASK-0050): `subscribe` exposes push lifecycle, and `capabilities` and
    /// snapshots report one instance identity.
    #[allow(clippy::too_many_arguments)]
    pub fn new_arc_with_subscription(
        worker: Arc<workers::Worker>,
        watches: Watches,
        control_socket: Option<PathBuf>,
        control_state: Arc<Mutex<WatcherState>>,
        coordinator: Option<Arc<AwaitCoordinator>>,
        outputs: Option<Arc<OutputRegistry>>,
        instance: Arc<WatcherInstance>,
        broker: Option<Arc<SnapshotBroker>>,
        recorder: Option<Arc<DurationRecorder>>,
    ) -> Arc<Self> {
        Self::new_arc_with_shared(
            worker,
            Arc::new(std::sync::Mutex::new(watches)),
            control_socket,
            control_state,
            coordinator,
            outputs,
            instance,
            broker,
            recorder,
            None,
        )
    }

    /// Creates the strategy around the SHARED watch config handle (TASK-0091,
    /// AC6/AC7): the reload transaction swaps this handle at commit, so every
    /// target/emit/estimate decision after a valid reload reflects the new
    /// revision under one lock — never a stale private copy.
    #[allow(clippy::too_many_arguments)]
    pub fn new_arc_with_shared(
        worker: Arc<workers::Worker>,
        shared: Arc<std::sync::Mutex<Watches>>,
        control_socket: Option<PathBuf>,
        control_state: Arc<Mutex<WatcherState>>,
        coordinator: Option<Arc<AwaitCoordinator>>,
        outputs: Option<Arc<OutputRegistry>>,
        instance: Arc<WatcherInstance>,
        broker: Option<Arc<SnapshotBroker>>,
        recorder: Option<Arc<DurationRecorder>>,
        lifecycle: Option<Arc<crate::config_lifecycle::ConfigLifecycle>>,
    ) -> Arc<Self> {
        let strategy = Arc::new(NonBlockStrategy {
            worker,
            shared,
            control_socket,
            control_state,
            schedule_lock: Mutex::new(()),
            retry_cache: Mutex::new(std::collections::BTreeMap::new()),
            generation_filepaths: Mutex::new(std::collections::BTreeMap::new()),
            coordinator,
            outputs,
            instance,
            broker,
            control_server: Mutex::new(None),
            pending_old_server: Mutex::new(None),
            self_arc: Mutex::new(None),
            recorder,
            lifecycle,
        });
        *strategy.self_arc.lock().unwrap() = Some(Arc::clone(&strategy));
        strategy
    }

    /// Publishes the control socket when configured.
    ///
    /// The control socket is an auxiliary control surface, so a bind failure
    /// (for example, another live instance already owns the socket) must NOT
    /// bring the watcher down. We log a warning and continue without it.
    pub fn start_control_server(&self) -> Option<ControlServer> {
        let path = self.control_socket.as_ref()?;
        match self.build_control_server(path) {
            Ok(server) => {
                stdout::info(&format!("Control socket listening at {}", path.display()));
                Some(server)
            }
            Err(err) => {
                stdout::warn(&format!(
                    "Control socket unavailable at {}: {}. Continuing without it.",
                    path.display(),
                    err
                ));
                None
            }
        }
    }

    /// Builds a control server bound to `path` with the current targets and
    /// the same orchestration closures as startup (TASK-0090 AC8). Targets
    /// and estimates are resolved from the SHARED config at request time
    /// (TASK-0091, AC6), so a valid reload is served by the same server
    /// without a rebuild.
    fn build_control_server(&self, path: &Path) -> Result<ControlServer, String> {
        let runner = self
            .self_arc
            .lock()
            .unwrap()
            .clone()
            .expect("self arc set by new_arc");
        let targets = self
            .shared
            .lock()
            .unwrap()
            .targets()
            .into_iter()
            .map(|rule| {
                let commands = rule.commands();
                ControlTarget {
                    name: rule.name,
                    commands,
                }
            })
            .collect();

        let run_runner = Arc::clone(&runner);
        let retry_runner = Arc::clone(&runner);
        let emit_runner = Arc::clone(&runner);
        let cancel_runner = Arc::clone(&runner);
        let coordinator = self.coordinator.clone();
        let outputs = self.outputs.clone();
        let instance = Arc::clone(&self.instance);
        let broker = self.broker.clone();
        // TASK-0055: estimate provider computed at request time from the
        // target's resolved plan signature; None when no recorder is wired.
        // Reads the shared config so estimates reflect the live revision
        // after a reload (TASK-0091, AC6).
        let estimates: Option<TargetEstimateProvider> = self.recorder.as_ref().map(|recorder| {
            let shared = Arc::clone(&self.shared);
            let recorder = Arc::clone(recorder);
            let concurrency = self.worker.concurrency();
            let fail_fast = self.worker.fail_fast();
            let root = self.shared.lock().unwrap().root().to_path_buf();
            let provider: TargetEstimateProvider = Arc::new(move |target: &ControlTarget| {
                let plan = shared.lock().unwrap().target_plan(&target.name)?;
                let plan = path_context::resolve_context(&plan, &root).ok()?;
                let signature = plan.execution_signature(concurrency, fail_fast);
                recorder.estimate(&signature, None)
            });
            provider
        });
        let mut api = ControlApi::new(Arc::clone(&self.control_state))
            .with_targets(targets)
            .with_run(move |target, sequential| run_runner.run_target(&target, sequential))
            .with_retry(move |generation| retry_runner.retry_generation(generation))
            .with_emit(move |path| emit_runner.emit_path(&path))
            .with_cancel(move |generation| cancel_runner.cancel_generation(generation))
            .with_instance(instance);
        if let Some(coordinator) = coordinator {
            api = api.with_awaiting(coordinator);
        }
        if let Some(outputs) = outputs {
            api = api.with_outputs(outputs);
        }
        if let Some(broker) = broker {
            api = api.with_snapshots(broker);
        }
        if let Some(estimates) = estimates {
            api = api.with_estimates(estimates);
        }
        if let Some(lifecycle) = self.lifecycle.clone() {
            // TASK-0091 AC3/AC6: config lifecycle and target lookup both read
            // live shared state after reload without rebuilding server.
            let shared = Arc::clone(&self.shared);
            let provider: crate::control::TargetsProvider = Arc::new(move || {
                shared
                    .lock()
                    .unwrap()
                    .targets()
                    .into_iter()
                    .map(|rule| {
                        let commands = rule.commands();
                        ControlTarget {
                            name: rule.name,
                            commands,
                        }
                    })
                    .collect()
            });
            api = api
                .with_targets_provider(provider)
                .with_lifecycle(lifecycle);
        }
        ControlServer::bind(path, api).map_err(|err| err.to_string())
    }

    /// AC8 socket-path handoff, prepare phase: binds a NEW server at
    /// `new_path` BEFORE the config commit and parks the current server for
    /// retirement. A bind failure returns an error — the reload takes the
    /// invalid fatal path (never a silent stale socket).
    pub fn prepare_socket_swap(&self, new_path: PathBuf) -> Result<(), String> {
        let new_server = self.build_control_server(&new_path)?;
        let old_server = self.control_server.lock().unwrap().replace(new_server);
        *self.pending_old_server.lock().unwrap() = old_server;
        stdout::info(&format!(
            "Control socket rebinding to {} (old socket retired after commit).",
            new_path.display()
        ));
        Ok(())
    }

    /// AC8 socket-path handoff, retire phase: drops the OLD server (its
    /// socket file is removed) after the commit boundary.
    pub fn retire_socket_swap(&self) {
        if let Some(old) = self.pending_old_server.lock().unwrap().take() {
            drop(old);
        }
    }

    fn remember_filepath(&self, generation: u64, filepath: Option<String>) {
        let mut filepaths = self.generation_filepaths.lock().unwrap();
        filepaths.insert(generation, filepath);
        while filepaths.len() > 256 {
            let Some(oldest) = filepaths.keys().next().copied() else {
                break;
            };
            filepaths.remove(&oldest);
        }
    }

    /// Test seam mirroring the retained trigger path of a completed run.
    #[cfg(test)]
    fn remember_filepath_for_test(&self, generation: u64, filepath: Option<&str>) {
        self.remember_filepath(generation, filepath.map(str::to_owned));
    }

    /// Cancels an exact generation through the worker run contract
    /// (TASK-0046): a compare-and-act on generation identity. Returns the
    /// disposition or a no-op for already-terminal/unknown generations.
    pub fn cancel_generation(
        &self,
        generation: u64,
    ) -> Result<crate::workers::CancelResult, String> {
        self.worker.cancel_generation(generation)
    }

    /// Retries failed finite jobs from the latest terminal generation, or
    /// requires one exact source. Resolve the default under the same schedule
    /// lock as selection so another run cannot change it between requests.
    pub fn retry_generation(
        &self,
        requested_generation: Option<u64>,
    ) -> Result<crate::control::RetryRun, crate::control::ControlRetryError> {
        if let Some(retry) = requested_generation
            .and_then(|generation| self.retry_cache.lock().unwrap().get(&generation).cloned())
        {
            return Ok(retry);
        }

        let shared = self.shared.lock().unwrap();
        let _schedule = self.schedule_lock.lock().unwrap();
        let state = self.control_state.lock().unwrap();
        let source_generation = requested_generation.unwrap_or_else(|| state.generation());
        if let Some(retry) = self
            .retry_cache
            .lock()
            .unwrap()
            .get(&source_generation)
            .cloned()
        {
            return Ok(retry);
        }
        if state.generation() == 0 {
            return Err(crate::control::ControlRetryError::SourceUnavailable {
                generation: source_generation,
            });
        }
        let debounce_pending = self
            .coordinator
            .as_ref()
            .is_some_and(|coordinator| !coordinator.snapshot_facts().2.is_empty());
        let worker_busy = self.worker.has_active_or_queued_run();
        if state.generation() != source_generation {
            return Err(if worker_busy || debounce_pending {
                crate::control::ControlRetryError::WatcherBusy
            } else {
                crate::control::ControlRetryError::SourceUnavailable {
                    generation: source_generation,
                }
            });
        }
        if matches!(
            state.state(),
            crate::watcher_state::WatcherExecutionState::Running
                | crate::watcher_state::WatcherExecutionState::Cancelled
        ) {
            return Err(crate::control::ControlRetryError::SourceNotTerminal {
                generation: source_generation,
            });
        }
        if state.superseded_by().is_some() {
            return Err(crate::control::ControlRetryError::SourceUnavailable {
                generation: source_generation,
            });
        }

        let revision = shared.revision().cloned();
        let current_revision = revision.as_ref().map(|revision| revision.number);
        if state.revision() != current_revision {
            return Err(crate::control::ControlRetryError::StaleRevision {
                source: state.revision(),
                current: current_revision,
            });
        }
        let selected = crate::retry::retryable_task_names(state.tasks()).map_err(|_| {
            crate::control::ControlRetryError::NoRetryableJobs {
                generation: source_generation,
            }
        })?;

        let plan = shared.retry_plan(&selected);
        if plan.is_empty() || plan.task_names() != selected {
            return Err(crate::control::ControlRetryError::SourceUnavailable {
                generation: source_generation,
            });
        }
        for task in plan.stages.iter().flat_map(|stage| match stage {
            crate::plan::Stage::Serial(task) => vec![task],
            crate::plan::Stage::Parallel { tasks, .. } => tasks.iter().collect(),
        }) {
            if task.service {
                return Err(crate::control::ControlRetryError::UnsupportedService {
                    name: task.name.clone(),
                });
            }
        }

        if worker_busy || debounce_pending {
            return Err(crate::control::ControlRetryError::WatcherBusy);
        }

        let filepath = self
            .generation_filepaths
            .lock()
            .unwrap()
            .get(&source_generation)
            .cloned()
            .ok_or(crate::control::ControlRetryError::SourceUnavailable {
                generation: source_generation,
            })?;
        let changed = state.changed().to_vec();
        let run_id = self
            .worker
            .schedule_retry(
                plan,
                source_generation,
                filepath.as_deref(),
                changed,
                revision.clone(),
            )
            .map_err(crate::control::ControlRetryError::Internal)?;
        let retry = crate::control::RetryRun {
            source_generation,
            run_id,
            tasks: selected,
            revision: current_revision,
            revision_hash: revision.map(|revision| revision.hash),
        };
        self.remember_filepath(run_id, filepath);
        let mut cache = self.retry_cache.lock().unwrap();
        cache.insert(source_generation, retry.clone());
        while cache.len() > 256 {
            let Some(oldest) = cache.keys().next().copied() else {
                break;
            };
            cache.remove(&oldest);
        }
        drop(cache);
        drop(state);
        drop(_schedule);
        drop(shared);
        Ok(retry)
    }

    /// Runs a control-requested target through the worker run contract:
    /// cancel the active run, then schedule the target's rules with its
    /// structural target identity and execution signature (TASK-0054).
    /// `sequential` (TASK-0073) requests effective concurrency one for this
    /// exact generation only; later native generations keep their configured
    /// concurrency.
    ///
    /// The plan and its frozen config revision are read under ONE shared
    /// lock (TASK-0091, AC7): a run concurrent with reload binds to exactly
    /// one revision; a target that left the revision is a typed
    /// `TargetNotFound` outcome.
    pub fn run_target(
        &self,
        target: &str,
        sequential: bool,
    ) -> Result<ScheduledRun, ControlRunError> {
        let (plan, revision) = {
            let shared = self.shared.lock().unwrap();
            match shared.target_plan(target) {
                Some(plan) => (plan, shared.revision().cloned()),
                None => {
                    return Err(ControlRunError::TargetNotFound {
                        target: target.to_owned(),
                    })
                }
            }
        };
        let _schedule = self.schedule_lock.lock().unwrap();
        let commands = plan.commands().len();
        self.worker
            .cancel_running_tasks()
            .map_err(ControlRunError::Internal)?;
        let run_id = self
            .worker
            .schedule_target(plan, target, sequential, revision.clone())
            .map_err(ControlRunError::Internal)?;
        self.remember_filepath(run_id, None);
        diagnostics::debug(&diagnostics::Record {
            source: Some("control"),
            decision: Some("scheduled"),
            generation: Some(run_id),
            policy: Some("restart"),
            commands: Some(commands),
            task: Some(target.to_owned()),
            ..Default::default()
        });
        Ok(ScheduledRun {
            run_id,
            revision: revision.as_ref().map(|r| r.number),
            revision_hash: revision.map(|r| r.hash),
        })
    }

    /// Routes one synthetic path change through the exact shared policy used
    /// for native filesystem events: `watch_plan` (normalization, change
    /// match, ignore precedence, task ordering, `run_on_init` exclusions),
    /// then the same cancel-and-schedule busy-run contract. The plan and its
    /// frozen config revision are read under ONE shared lock (TASK-0091,
    /// AC7), so an emit concurrent with reload binds to exactly one revision.
    /// Unmatched and ignored paths are explicit outcomes with no scheduled
    /// generation.
    pub fn emit_path(&self, path: &str) -> Result<EmitOutcome, String> {
        let (matched, plan, revision) = {
            let shared = self.shared.lock().unwrap();
            match shared.watch_plan(path) {
                Some(plan) => (plan.task_names(), plan, shared.revision().cloned()),
                None => {
                    let explained = shared.explain(path);
                    diagnostics::debug(&diagnostics::Record {
                        source: Some("control"),
                        decision: Some(if explained.ignored.is_empty() {
                            "unmatched"
                        } else {
                            "ignored"
                        }),
                        path: Some(path.to_owned()),
                        normalized: Some(shared.normalized_path(path)),
                        ..Default::default()
                    });
                    return Ok(if explained.ignored.is_empty() {
                        EmitOutcome::unmatched()
                    } else {
                        EmitOutcome::ignored()
                    });
                }
            }
        };
        let _schedule = self.schedule_lock.lock().unwrap();
        let commands = plan.commands().len();
        self.worker.cancel_running_tasks()?;
        let run_id = self.worker.schedule_plan_correlated(
            plan,
            path,
            Some(path),
            None,
            vec![],
            revision.clone(),
        )?;
        self.remember_filepath(run_id, Some(path.to_owned()));
        diagnostics::debug(&diagnostics::Record {
            source: Some("control"),
            decision: Some("scheduled"),
            generation: Some(run_id),
            policy: Some("restart"),
            commands: Some(commands),
            path: Some(path.to_owned()),
            ..Default::default()
        });
        Ok(EmitOutcome::scheduled_at(matched, run_id, revision))
    }
}

impl RunStrategy for NonBlockStrategy {
    fn policy(&self) -> &'static str {
        "restart"
    }

    fn on_ready(&self) {
        if let Some(server) = self.start_control_server() {
            *self.control_server.lock().unwrap() = Some(server);
        }
    }

    fn run_init(&self, plan: RunPlan, revision: Option<ConfigRevision>) -> Option<u64> {
        let _schedule = self.schedule_lock.lock().unwrap();
        match self.worker.schedule_plan(plan, "", revision) {
            Ok(run_id) => {
                self.remember_filepath(run_id, None);
                Some(run_id)
            }
            Err(err) => {
                stdout::error(&format!("failed to initiate next run: {:?}", err));
                None
            }
        }
    }

    fn on_batch(&self, batch: &Batch) {
        if let Some(coordinator) = &self.coordinator {
            coordinator.note_batch(batch.id.0);
        }
    }

    fn on_batch_complete(&self) {
        if let Some(coordinator) = &self.coordinator {
            coordinator.note_batch_complete();
        }
    }

    fn run_change(
        &self,
        plan: RunPlan,
        filepath: &str,
        batch: &Batch,
        revision: Option<ConfigRevision>,
    ) -> Option<u64> {
        let _schedule = self.schedule_lock.lock().unwrap();
        if let Err(err) = self.worker.cancel_running_tasks() {
            stdout::error(&format!(
                "failed to cancel current running tasks: {:?}",
                err
            ));
        }

        let commands = plan.commands().len();
        match self.worker.schedule_plan_correlated(
            plan,
            filepath,
            Some(filepath),
            Some(batch.id.0),
            batch.changed.clone(),
            revision,
        ) {
            Ok(run_id) => {
                self.remember_filepath(run_id, Some(filepath.to_owned()));
                diagnostics::debug(&diagnostics::Record {
                    batch: Some(batch.id.0),
                    source: Some("filesystem"),
                    decision: Some("scheduled"),
                    generation: Some(run_id),
                    policy: Some("restart"),
                    commands: Some(commands),
                    ..Default::default()
                });
                Some(run_id)
            }
            Err(err) => {
                stdout::error(&format!("failed to initiate next run: {:?}", err));
                None
            }
        }
    }

    fn run_manual(&self, plan: RunPlan, revision: Option<ConfigRevision>) -> Option<u64> {
        let _schedule = self.schedule_lock.lock().unwrap();
        match self
            .worker
            .schedule_plan_correlated(plan, "keyboard", None, None, vec![], revision)
        {
            Ok(run_id) => {
                self.remember_filepath(run_id, None);
                Some(run_id)
            }
            Err(error) => {
                stdout::error(&format!("failed to initiate keyboard run: {:?}", error));
                None
            }
        }
    }

    fn retry_latest(&self, _watches: &Arc<Mutex<Watches>>) -> Option<u64> {
        match self.retry_generation(None) {
            Ok(retry) => {
                stdout::info("Retrying failed jobs from keyboard shortcut.");
                Some(retry.run_id)
            }
            Err(error) => {
                present_retry_rejection(&error);
                None
            }
        }
    }

    fn is_running(&self) -> bool {
        self.control_state.lock().unwrap().is_running()
    }

    fn is_generation_complete(&self, generation: u64) -> bool {
        let state = self.control_state.lock().unwrap();
        state.generation() >= generation && !state.is_running()
    }
}

#[cfg(test)]
mod tests {
    use super::init_action;
    use super::InitAction;
    use super::NonBlockStrategy;
    use super::RunStrategy;
    use crate::control::{ControlApi, ControlRunError, ControlServer};
    use crate::plan::RunPlan;
    use crate::rules::Rules;
    use crate::watcher_state::WatcherState;
    use crate::watches::Watches;
    use crate::workers;
    use std::sync::{Arc, Mutex};

    fn unique_socket(label: &str) -> std::path::PathBuf {
        std::env::temp_dir().join(format!("funzzy-wnb-{}-{}.sock", std::process::id(), label))
    }

    fn rule(name: &str) -> Rules {
        Rules::new(
            name.to_owned(),
            vec!["echo hi".to_owned()],
            vec!["src/**".to_owned()],
            vec![],
            false,
        )
    }

    fn rule_with_ignore(name: &str) -> Rules {
        Rules::new(
            name.to_owned(),
            vec!["echo hi".to_owned()],
            vec!["src/**".to_owned()],
            vec!["src/generated/**".to_owned()],
            false,
        )
    }

    #[test]
    fn it_runs_init_rules_when_flag_is_enabled_and_rules_exist() {
        assert!(matches!(
            init_action(Some(RunPlan::from_rules(vec![rule("build")])), true),
            InitAction::Run(_)
        ));
    }

    #[test]
    fn it_waits_when_run_on_init_flag_is_disabled() {
        assert!(matches!(
            init_action(Some(RunPlan::from_rules(vec![rule("build")])), false),
            InitAction::Wait
        ));
    }

    #[test]
    fn it_waits_when_no_init_rules_exist() {
        assert!(matches!(init_action(None, true), InitAction::Wait));
        assert!(matches!(init_action(None, false), InitAction::Wait));
    }

    #[test]
    fn it_continues_without_a_control_socket_when_it_is_already_in_use() {
        let path = unique_socket("conflict");

        // A live instance already owns the socket.
        let holder_state = Arc::new(Mutex::new(WatcherState::default()));
        let _holder = ControlServer::bind(&path, ControlApi::new(holder_state)).unwrap();

        let worker = Arc::new(workers::Worker::new(false, false, |_| {}));
        let control_state = Arc::new(Mutex::new(WatcherState::default()));
        let strategy = NonBlockStrategy::new_arc(
            worker,
            Watches::new(vec![]),
            Some(path.clone()),
            control_state,
            None,
            None,
        );

        // The watcher must NOT die just because its control socket is taken.
        let server = strategy.start_control_server();
        assert!(
            server.is_none(),
            "control server startup must be non-fatal when the socket is already in use"
        );
    }

    #[test]
    fn it_starts_the_control_server_when_the_socket_is_free() {
        let path = unique_socket("free");

        let worker = Arc::new(workers::Worker::new(false, false, |_| {}));
        let control_state = Arc::new(Mutex::new(WatcherState::default()));
        let strategy = NonBlockStrategy::new_arc(
            worker,
            Watches::new(vec![]),
            Some(path.clone()),
            control_state,
            None,
            None,
        );

        let server = strategy.start_control_server();
        assert!(
            server.is_some(),
            "control server should start when the socket is free"
        );
        assert!(path.exists(), "the socket file should be created");
    }

    #[test]
    fn it_runs_a_control_target_through_the_worker_contract() {
        let watches = Watches::new(vec![rule("my tests")]);
        let worker = Arc::new(workers::Worker::new(false, false, |_| {}));
        let control_state = Arc::new(Mutex::new(WatcherState::default()));
        let strategy = NonBlockStrategy::new_arc(worker, watches, None, control_state, None, None);

        let scheduled = strategy
            .run_target("my tests", false)
            .expect("known target should schedule");
        assert_eq!(scheduled.run_id, 1);
        assert_eq!(scheduled.revision, None, "legacy shape has no revision");
    }

    #[test]
    fn it_rejects_unknown_control_targets() {
        let watches = Watches::new(vec![rule("my tests")]);
        let worker = Arc::new(workers::Worker::new(false, false, |_| {}));
        let control_state = Arc::new(Mutex::new(WatcherState::default()));
        let strategy = NonBlockStrategy::new_arc(worker, watches, None, control_state, None, None);

        let err = strategy
            .run_target("nope", false)
            .expect_err("unknown target");
        // TASK-0091, AC7: a stale/unknown target is an actionable typed
        // outcome, never a message an agent would have to parse.
        let ControlRunError::TargetNotFound { target } = err else {
            panic!("expected target_not_found, got {err:?}")
        };
        assert_eq!(target, "nope");
        let (code, message, _) = ControlRunError::TargetNotFound {
            target: "nope".to_owned(),
        }
        .to_rpc();
        assert_eq!(code, -32016);
        assert_eq!(message, "target_not_found");
    }

    #[test]
    fn it_emits_a_path_through_the_worker_contract() {
        let watches = Watches::new(vec![rule("my tests")]);
        let worker = Arc::new(workers::Worker::new(false, false, |_| {}));
        let control_state = Arc::new(Mutex::new(WatcherState::default()));
        let strategy = NonBlockStrategy::new_arc(worker, watches, None, control_state, None, None);

        let outcome = strategy
            .emit_path("src/main.rs")
            .expect("matched path should schedule");
        assert_eq!(outcome.outcome, "scheduled");
        assert_eq!(outcome.matched, vec!["my tests".to_owned()]);
        assert_eq!(outcome.run_id, Some(1));
    }

    #[test]
    fn it_emits_an_absolute_path_identically_to_a_relative_one() {
        let watches = Watches::new(vec![rule("my tests")]);
        let worker = Arc::new(workers::Worker::new(false, false, |_| {}));
        let control_state = Arc::new(Mutex::new(WatcherState::default()));
        let strategy = NonBlockStrategy::new_arc(worker, watches, None, control_state, None, None);

        let relative = strategy.emit_path("src/main.rs").expect("relative");
        let root = strategy.shared.lock().unwrap().root().to_path_buf();
        let absolute = strategy
            .emit_path(&format!("{}/src/main.rs", root.display()))
            .expect("absolute");
        assert_eq!(relative.outcome, "scheduled");
        assert_eq!(absolute.outcome, "scheduled");
        assert_eq!(absolute.matched, vec!["my tests".to_owned()]);
    }

    #[test]
    fn it_reports_unmatched_emit_without_scheduling() {
        let watches = Watches::new(vec![rule("my tests")]);
        let worker = Arc::new(workers::Worker::new(false, false, |_| {}));
        let control_state = Arc::new(Mutex::new(WatcherState::default()));
        let strategy = NonBlockStrategy::new_arc(worker, watches, None, control_state, None, None);

        let outcome = strategy
            .emit_path("docs/readme.md")
            .expect("unmatched must not fail");
        assert_eq!(outcome.outcome, "unmatched");
        assert!(outcome.matched.is_empty());
        assert_eq!(outcome.run_id, None);
    }

    #[test]
    fn it_reports_ignored_emit_without_scheduling() {
        let watches = Watches::new(vec![rule_with_ignore("my tests")]);
        let worker = Arc::new(workers::Worker::new(false, false, |_| {}));
        let control_state = Arc::new(Mutex::new(WatcherState::default()));
        let strategy = NonBlockStrategy::new_arc(worker, watches, None, control_state, None, None);

        let outcome = strategy
            .emit_path("src/generated/out.rs")
            .expect("ignored must not fail");
        assert_eq!(outcome.outcome, "ignored");
        assert!(outcome.matched.is_empty());
        assert_eq!(outcome.run_id, None);
    }

    #[test]
    fn retry_rejects_pending_batch_without_scheduling_work() {
        use crate::awaiting::AwaitCoordinator;
        use crate::task_result::{TaskSnapshot, TaskState};

        let watches = Watches::new(vec![rule("my tests")]);
        let worker = Arc::new(workers::Worker::new(false, false, |_| {}));
        let control_state = Arc::new(Mutex::new(WatcherState::default()));
        control_state
            .lock()
            .unwrap()
            .apply(crate::executor::Event::Started {
                run_id: 1,
                trigger: "src/main.rs".to_owned(),
                batch: Some(8),
                predecessor: None,
                changed: vec!["src/main.rs".to_owned()],
                commands: vec!["false".to_owned()],
                target: None,
                execution_signature: None,
                effective_concurrency: None,
                concurrency_source: None,
                revision: None,
                revision_hash: None,
            });
        control_state
            .lock()
            .unwrap()
            .apply(crate::executor::Event::TaskTerminal {
                run_id: 1,
                task: TaskSnapshot {
                    position: 0,
                    id: "my tests".to_owned(),
                    name: "my tests".to_owned(),
                    state: TaskState::Failed,
                    duration_ms: Some(1),
                },
            });
        control_state
            .lock()
            .unwrap()
            .apply(crate::executor::Event::Finished {
                run_id: 1,
                superseded_by: None,
                elapsed: std::time::Duration::from_millis(1),
                failures: vec!["my tests failed".to_owned()],
            });
        let coordinator = Arc::new(AwaitCoordinator::new());
        coordinator.note_batch(9);
        let strategy = NonBlockStrategy::new_arc(
            worker.clone(),
            watches,
            None,
            control_state,
            Some(coordinator),
            None,
        );

        assert!(matches!(
            strategy.retry_generation(Some(1)),
            Err(crate::control::ControlRetryError::WatcherBusy)
        ));
        assert!(!worker.has_active_or_queued_run());
    }

    #[test]
    fn retry_rejects_a_source_from_an_old_config_revision() {
        use crate::task_result::{TaskSnapshot, TaskState};

        let watches = Watches::new(vec![rule("my tests")]).with_revision(
            crate::config_revision::ConfigRevision {
                number: 2,
                hash: "revision-2".to_owned(),
            },
        );
        let worker = Arc::new(workers::Worker::new(false, false, |_| {}));
        let control_state = Arc::new(Mutex::new(WatcherState::default()));
        control_state
            .lock()
            .unwrap()
            .apply(crate::executor::Event::Started {
                run_id: 1,
                trigger: "src/main.rs".to_owned(),
                batch: Some(8),
                predecessor: None,
                changed: vec!["src/main.rs".to_owned()],
                commands: vec!["false".to_owned()],
                target: None,
                execution_signature: None,
                effective_concurrency: None,
                concurrency_source: None,
                revision: Some(1),
                revision_hash: Some("revision-1".to_owned()),
            });
        control_state
            .lock()
            .unwrap()
            .apply(crate::executor::Event::TaskTerminal {
                run_id: 1,
                task: TaskSnapshot {
                    position: 0,
                    id: "my tests".to_owned(),
                    name: "my tests".to_owned(),
                    state: TaskState::Failed,
                    duration_ms: Some(1),
                },
            });
        control_state
            .lock()
            .unwrap()
            .apply(crate::executor::Event::Finished {
                run_id: 1,
                superseded_by: None,
                elapsed: std::time::Duration::from_millis(1),
                failures: vec!["my tests failed".to_owned()],
            });
        let strategy =
            NonBlockStrategy::new_arc(worker.clone(), watches, None, control_state, None, None);

        assert!(matches!(
            strategy.retry_generation(Some(1)),
            Err(crate::control::ControlRetryError::StaleRevision {
                source: Some(1),
                current: Some(2),
            })
        ));
        assert!(!worker.has_active_or_queued_run());
    }

    #[test]
    fn it_marks_pending_debounce_through_the_coordinator() {
        use crate::awaiting::AwaitCoordinator;
        use crate::identity::Batch;

        let watches = Watches::new(vec![rule("my tests")]);
        let worker = Arc::new(workers::Worker::new(false, false, |_| {}));
        let control_state = Arc::new(Mutex::new(WatcherState::default()));
        let coordinator = Arc::new(AwaitCoordinator::new());
        let strategy = NonBlockStrategy::new_arc(
            worker,
            watches,
            None,
            control_state,
            Some(coordinator.clone()),
            None,
        );

        let batch = Batch::normalized(crate::identity::BatchId(9), vec!["src/main.rs".to_owned()]);
        strategy.on_batch(&batch);
        strategy.on_batch_complete();

        // The coordinator observed the open window and its completion; the
        // await surface can classify freshness around pending debounce.
        let result = coordinator.await_generation(
            crate::awaiting::AwaitMode::Exact(1),
            std::time::Duration::from_millis(10),
            &Arc::new(Mutex::new(WatcherState::default())),
            None,
            None,
            "fz-test",
            None,
        );
        assert_eq!(result.latest_batch, Some(9));
        assert!(!result.pending_work.debounce_active);
    }

    #[test]
    fn run_target_reads_the_shared_config_so_reload_is_served_by_the_same_strategy() {
        // TASK-0091, AC6/AC7: the strategy resolves targets from the SHARED
        // watch config, so after a reload commit (shared swap + new revision)
        // the same strategy schedules the new job and reports its revision —
        // never a stale private copy.
        let root = std::env::temp_dir().join(format!("fzz-shared-{}", std::process::id()));
        std::fs::create_dir_all(&root).unwrap();
        let shared = Arc::new(Mutex::new(
            Watches::with_root_and_concurrency(vec![rule("build")], root.clone(), 2).with_revision(
                crate::config_revision::ConfigRevision {
                    number: 1,
                    hash: "hash-1".to_owned(),
                },
            ),
        ));
        let worker = Arc::new(workers::Worker::with_root_and_concurrency(
            false,
            false,
            root.clone(),
            2,
            |_| {},
        ));
        let control_state = Arc::new(Mutex::new(WatcherState::default()));
        let strategy = NonBlockStrategy::new_arc_with_shared(
            worker,
            Arc::clone(&shared),
            None,
            control_state,
            None,
            None,
            Arc::new(crate::watcher_state::WatcherInstance::new()),
            None,
            None,
            None,
        );

        // Before the reload: build is the only target, frozen under revision 1.
        let first = strategy
            .run_target("build", false)
            .expect("build exists at rev 1");
        assert_eq!(first.run_id, 1);
        assert_eq!(first.revision, Some(1));
        assert_eq!(first.revision_hash.as_deref(), Some("hash-1"));
        assert!(matches!(
            strategy.run_target("lint", false),
            Err(crate::control::ControlRunError::TargetNotFound { .. })
        ));

        // Reload commit: swap the shared config to revision 2 with a new job.
        shared.lock().unwrap().swap_config(
            Watches::with_root_and_concurrency(vec![rule("build"), rule("lint")], root.clone(), 2)
                .with_revision(crate::config_revision::ConfigRevision {
                    number: 2,
                    hash: "hash-2".to_owned(),
                }),
        );

        // The same strategy now serves the new revision: lint schedules under
        // revision 2, and build too.
        let lint = strategy
            .run_target("lint", false)
            .expect("lint exists after reload");
        assert_eq!(lint.run_id, 2);
        assert_eq!(lint.revision, Some(2));
        assert_eq!(lint.revision_hash.as_deref(), Some("hash-2"));
        let build = strategy
            .run_target("build", false)
            .expect("build still exists");
        assert_eq!(build.revision, Some(2));
        std::fs::remove_dir_all(&root).unwrap();
    }

    #[test]
    fn emit_binds_to_the_shared_config_revision_after_reload() {
        // TASK-0091, AC7: emit routes under the shared config and reports the
        // frozen revision it scheduled under — a reload swap is observed by
        // the next emit without any strategy rebuild.
        let root = std::env::temp_dir().join(format!("fzz-emit-shared-{}", std::process::id()));
        std::fs::create_dir_all(root.join("src")).unwrap();
        std::fs::create_dir_all(root.join("docs")).unwrap();
        let shared = Arc::new(Mutex::new(
            Watches::with_root_and_concurrency(
                vec![Rules::new(
                    "build".to_owned(),
                    vec!["echo hi".to_owned()],
                    vec!["src/**".to_owned()],
                    vec![],
                    false,
                )],
                root.clone(),
                2,
            )
            .with_revision(crate::config_revision::ConfigRevision {
                number: 1,
                hash: "hash-1".to_owned(),
            }),
        ));
        let worker = Arc::new(workers::Worker::with_root_and_concurrency(
            false,
            false,
            root.clone(),
            2,
            |_| {},
        ));
        let control_state = Arc::new(Mutex::new(WatcherState::default()));
        let strategy = NonBlockStrategy::new_arc_with_shared(
            worker,
            Arc::clone(&shared),
            None,
            control_state,
            None,
            None,
            Arc::new(crate::watcher_state::WatcherInstance::new()),
            None,
            None,
            None,
        );

        let first = strategy
            .emit_path("src/a.rs")
            .expect("src matches at rev 1");
        assert_eq!(first.outcome, "scheduled");
        assert_eq!(first.revision, Some(1));

        // Reload: the docs job joins under revision 2.
        shared.lock().unwrap().swap_config(
            Watches::with_root_and_concurrency(
                vec![
                    Rules::new(
                        "build".to_owned(),
                        vec!["echo hi".to_owned()],
                        vec!["src/**".to_owned()],
                        vec![],
                        false,
                    ),
                    Rules::new(
                        "docs".to_owned(),
                        vec!["echo docs".to_owned()],
                        vec!["docs/**".to_owned()],
                        vec![],
                        false,
                    ),
                ],
                root.clone(),
                2,
            )
            .with_revision(crate::config_revision::ConfigRevision {
                number: 2,
                hash: "hash-2".to_owned(),
            }),
        );

        let docs = strategy
            .emit_path("docs/guide.md")
            .expect("docs matches after reload");
        assert_eq!(docs.outcome, "scheduled");
        assert_eq!(docs.matched, vec!["docs".to_owned()]);
        assert_eq!(docs.revision, Some(2));
        assert_eq!(docs.revision_hash.as_deref(), Some("hash-2"));

        // src still matches and now reports revision 2 too.
        let src = strategy
            .emit_path("src/a.rs")
            .expect("src matches after reload");
        assert_eq!(src.revision, Some(2));
        std::fs::remove_dir_all(&root).unwrap();
    }

    #[test]
    fn live_server_targets_reflect_the_shared_config_after_reload() {
        // TASK-0091, AC6: the running control server's `targets` method
        // resolves from the SHARED config at request time, so after a reload
        // swap the same server serves the new jobs.
        use std::io::BufRead as _;
        use std::io::Write as _;

        let root = std::env::temp_dir().join(format!("fzz-live-targets-{}", std::process::id()));
        std::fs::create_dir_all(root.join("src")).unwrap();
        let shared = Arc::new(Mutex::new(
            Watches::with_root_and_concurrency(vec![rule("build")], root.clone(), 2).with_revision(
                crate::config_revision::ConfigRevision {
                    number: 1,
                    hash: "hash-1".to_owned(),
                },
            ),
        ));
        let worker = Arc::new(workers::Worker::with_root_and_concurrency(
            false,
            false,
            root.clone(),
            2,
            |_| {},
        ));
        let control_state = Arc::new(Mutex::new(WatcherState::default()));
        let coordinator = Arc::new(crate::awaiting::AwaitCoordinator::new());
        let instance = Arc::new(crate::watcher_state::WatcherInstance {
            token: "fz-test".to_owned(),
            started_at_epoch_ms: 1,
        });
        let broker = Arc::new(crate::snapshot::SnapshotBroker::new(
            instance.as_ref().clone(),
            Arc::clone(&control_state),
            Arc::clone(&coordinator),
        ));
        let recorder = Arc::new(crate::duration_recorder::DurationRecorder::new(
            crate::duration_store::DurationStore::new(root.join("run-durations-v1.json")),
        ));
        let strategy = NonBlockStrategy::new_arc_with_shared(
            worker,
            Arc::clone(&shared),
            Some(root.join("sock")),
            control_state,
            Some(coordinator),
            Some(Arc::new(crate::output::OutputRegistry::new())),
            instance,
            Some(broker),
            Some(recorder),
            Some(Arc::new(crate::config_lifecycle::ConfigLifecycle::new())),
        );
        let server = strategy
            .start_control_server()
            .expect("control server starts");

        let list_targets = || -> Vec<String> {
            let mut stream =
                std::os::unix::net::UnixStream::connect(root.join("sock")).expect("connect");
            writeln!(
                stream,
                r#"{{"jsonrpc":"2.0","id":1,"method":"targets","params":{{}}}}"#
            )
            .expect("write");
            let mut line = String::new();
            std::io::BufReader::new(&mut stream)
                .read_line(&mut line)
                .expect("read");
            let value: serde_json::Value = serde_json::from_str(&line).expect("json");
            value["result"]
                .as_array()
                .unwrap()
                .iter()
                .filter_map(|t| t["name"].as_str().map(str::to_owned))
                .collect()
        };

        assert_eq!(list_targets(), vec!["build".to_owned()]);

        // Reload commit: swap the shared config to revision 2 with lint.
        shared.lock().unwrap().swap_config(
            Watches::with_root_and_concurrency(vec![rule("build"), rule("lint")], root.clone(), 2)
                .with_revision(crate::config_revision::ConfigRevision {
                    number: 2,
                    hash: "hash-2".to_owned(),
                }),
        );

        let targets = list_targets();
        assert_eq!(
            targets,
            vec!["build".to_owned(), "lint".to_owned()],
            "targets must reflect the reloaded shared config"
        );
        drop(server);
        std::fs::remove_dir_all(&root).unwrap();
    }

    #[test]
    fn it_does_not_schedule_when_only_ignored_rules_exist() {
        let watches = Watches::new(vec![rule_with_ignore("my tests")]);
        let worker = Arc::new(workers::Worker::new(false, false, |_| {}));
        let control_state = Arc::new(Mutex::new(WatcherState::default()));
        let strategy = NonBlockStrategy::new_arc(worker, watches, None, control_state, None, None);

        let outcome = strategy
            .emit_path("src/generated/out.rs")
            .expect("ignored path");
        assert_eq!(outcome.run_id, None);
    }
}

#[cfg(test)]
mod modification_gate_tests {
    use super::ModificationGate;
    use std::path::PathBuf;

    fn scratch(label: &str) -> PathBuf {
        let dir = std::env::temp_dir().join(format!(
            "fzz-gate-{}-{label}-{}",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        dir
    }

    /// A re-delivered event for an untouched file never routes twice
    /// (TASK-0114): the exact Linux chatter shape — same path, no write.
    #[test]
    fn redelivered_untouched_path_is_filtered() {
        let dir = scratch("chatter");
        let file = dir.join("a.rs");
        std::fs::write(&file, "one").unwrap();

        let mut gate = ModificationGate::new();
        let first = gate.changed(vec![file.display().to_string()]);
        assert_eq!(first.len(), 1, "first sighting routes");
        let second = gate.changed(vec![file.display().to_string()]);
        assert!(second.is_empty(), "re-delivery without a write is chatter");

        std::fs::remove_dir_all(&dir).unwrap();
    }

    /// A real rewrite routes again: mtime moved.
    #[test]
    fn rewritten_path_routes_again() {
        let dir = scratch("rewrite");
        let file = dir.join("a.rs");
        std::fs::write(&file, "one").unwrap();
        let mut gate = ModificationGate::new();
        gate.changed(vec![file.display().to_string()]);

        // Ensure the rewrite produces a distinct mtime (filesystems with
        // coarse timestamps): poll until it differs, bounded.
        let first_mtime = std::fs::metadata(&file).unwrap().modified().unwrap();
        loop {
            std::fs::write(&file, "two - longer content").unwrap();
            if std::fs::metadata(&file).unwrap().modified().unwrap() != first_mtime {
                break;
            }
            std::thread::sleep(std::time::Duration::from_millis(5));
        }

        let second = gate.changed(vec![file.display().to_string()]);
        assert_eq!(second.len(), 1, "a real rewrite routes");

        std::fs::remove_dir_all(&dir).unwrap();
    }

    /// Deletions never schedule; recreation afterwards routes once.
    #[test]
    fn deletion_is_quiet_but_recreation_routes() {
        let dir = scratch("delete");
        let file = dir.join("gone.rs");
        std::fs::write(&file, "x").unwrap();
        let mut gate = ModificationGate::new();
        gate.changed(vec![file.display().to_string()]);

        std::fs::remove_file(&file).unwrap();
        let deletion = gate.changed(vec![file.display().to_string()]);
        assert!(deletion.is_empty(), "deletion never schedules work");
        let still_gone = gate.changed(vec![file.display().to_string()]);
        assert!(still_gone.is_empty(), "absent path stays quiet");

        std::fs::write(&file, "recreated").unwrap();
        let recreated = gate.changed(vec![file.display().to_string()]);
        assert_eq!(recreated.len(), 1, "recreation routes once");

        std::fs::remove_dir_all(&dir).unwrap();
    }

    /// A fresh file that appeared between batches routes on first sighting.
    #[test]
    fn new_file_routes_on_first_sighting() {
        let dir = scratch("create");
        let file = dir.join("new.rs");
        std::fs::write(&file, "x").unwrap();
        let mut gate = ModificationGate::new();
        let routed = gate.changed(vec![file.display().to_string()]);
        assert_eq!(routed.len(), 1);

        std::fs::remove_dir_all(&dir).unwrap();
    }
}

#[cfg(test)]
mod modification_gate_seed_tests {
    use super::ModificationGate;

    fn scratch(label: &str) -> std::path::PathBuf {
        let dir = std::env::temp_dir().join(format!(
            "fzz-seed-{}-{label}-{}",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        dir
    }

    /// Pre-existing files are baselined at seed: synthesizing them later
    /// (Linux §4 parent-dir walk) can never route them (TASK-0114).
    #[test]
    fn pre_existing_files_never_route_after_seeding() {
        let dir = scratch("baseline");
        std::fs::create_dir_all(dir.join("workdir/backend")).unwrap();
        std::fs::write(dir.join("workdir/backend/test.rs"), "old").unwrap();

        let mut gate = ModificationGate::new();
        gate.seed(&[dir.join("workdir").display().to_string()]);

        let routed = gate.changed(vec![dir
            .join("workdir/backend/test.rs")
            .display()
            .to_string()]);
        assert!(
            routed.is_empty(),
            "baselined pre-existing file must not route on synthesis: {routed:?}"
        );

        std::fs::remove_dir_all(&dir).unwrap();
    }

    /// Exact-file patterns baseline only that file, never unrelated siblings.
    #[test]
    fn exact_file_seed_does_not_expand_to_its_parent() {
        let dir = scratch("exact-file");
        let manifest = dir.join("Cargo.toml");
        let unrelated = dir.join("target/debug/deps/stale.rcgu.o");
        std::fs::create_dir_all(unrelated.parent().unwrap()).unwrap();
        std::fs::write(&manifest, "[package]").unwrap();
        std::fs::write(&unrelated, "object").unwrap();

        let mut gate = ModificationGate::new();
        gate.seed(&[manifest.display().to_string()]);

        assert!(
            gate.changed(vec![manifest.display().to_string()])
                .is_empty(),
            "the exact configured file is baselined"
        );
        assert_eq!(
            gate.changed(vec![unrelated.display().to_string()]),
            vec![unrelated.display().to_string()],
            "an exact file baseline never traverses unrelated siblings"
        );

        std::fs::remove_dir_all(&dir).unwrap();
    }

    /// Broken symlink entries may disappear while a baseline is being read;
    /// they must be skipped without panicking or retrying forever.
    #[cfg(unix)]
    #[test]
    fn seed_ignores_broken_symlink_entries() {
        let dir = scratch("broken-symlink");
        let ordinary = dir.join("ordinary.txt");
        std::fs::write(&ordinary, "ordinary").unwrap();
        std::os::unix::fs::symlink(dir.join("missing"), dir.join("broken")).unwrap();

        let mut gate = ModificationGate::new();
        gate.seed(&[dir.display().to_string()]);
        assert!(
            gate.changed(vec![ordinary.display().to_string()])
                .is_empty(),
            "ordinary files remain baselined with broken entries"
        );

        std::fs::remove_dir_all(&dir).unwrap();
    }

    /// Files created AFTER seeding still route on first sighting.
    #[test]
    fn post_seed_creation_routes() {
        let dir = scratch("create-after-seed");
        std::fs::create_dir_all(dir.join("workdir")).unwrap();
        let mut gate = ModificationGate::new();
        gate.seed(&[dir.join("workdir").display().to_string()]);

        let file = dir.join("workdir/trigger.txt");
        std::fs::write(&file, "new").unwrap();
        let routed = gate.changed(vec![file.display().to_string()]);
        assert_eq!(routed.len(), 1, "post-seed creation routes");

        std::fs::remove_dir_all(&dir).unwrap();
    }

    /// Calling seed twice never overwrites a live baseline (fill-only).
    #[test]
    fn reseed_never_masks_inflight_writes() {
        let dir = scratch("reseed");
        let file = dir.join("a.txt");
        std::fs::create_dir_all(&dir).unwrap();
        std::fs::write(&file, "one").unwrap();

        let mut gate = ModificationGate::new();
        let roots = [dir.display().to_string()];
        gate.seed(&roots);
        gate.changed(vec![file.display().to_string()]); // now tracked

        // Rewrite bumps mtime past the baseline…
        std::fs::write(&file, "two-with-more-content").unwrap();
        // Calling seed again is fill-only; it keeps the older baseline.
        gate.seed(&roots);

        let routed = gate.changed(vec![file.display().to_string()]);
        assert_eq!(
            routed.len(),
            1,
            "fill-only re-seed keeps the older baseline"
        );

        std::fs::remove_dir_all(&dir).unwrap();
    }
}

#[cfg(test)]
mod keyboard_retry_tests {
    use super::{NonBlockStrategy, RunStrategy};
    use crate::rules::Rules;
    use crate::watcher_state::WatcherState;
    use crate::watches::Watches;
    use crate::workers;
    use std::sync::{Arc, Mutex};

    fn scratch(label: &str) -> std::path::PathBuf {
        std::env::temp_dir().join(format!("fzz-kbd-{}-{label}", std::process::id()))
    }

    fn rule(name: &str) -> Rules {
        Rules::new(
            name.to_owned(),
            vec!["echo hi".to_owned()],
            vec!["src/**".to_owned()],
            vec![],
            false,
        )
    }

    /// Ctrl-R on a blocking watch re-runs exactly the failed finite jobs of
    /// the latest terminal run, never the passed ones (KEYBOARD-RETRY).
    #[test]
    fn keyboard_retry_reruns_only_failed_blocking_jobs() {
        use super::BlockingStrategy;

        let dir = scratch("kbd-retry-blocking");
        std::fs::create_dir_all(&dir).unwrap();
        let fails = Rules::new(
            "fails".to_owned(),
            vec![
                "echo fail >> fails.log; if test -f allow; then exit 0; else exit 1; fi".to_owned(),
            ],
            vec!["src/**".to_owned()],
            vec![],
            false,
        );
        let passes = Rules::new(
            "passes".to_owned(),
            vec!["echo pass >> passes.log".to_owned()],
            vec!["src/**".to_owned()],
            vec![],
            false,
        );
        let watches = Arc::new(Mutex::new(Watches::new(vec![fails, passes])));
        let strategy = BlockingStrategy::new(dir.clone(), false, false, 1);

        let plan = watches
            .lock()
            .unwrap()
            .retry_plan(&["fails".to_owned(), "passes".to_owned()]);
        strategy.run_manual(plan, None);

        std::fs::write(dir.join("allow"), "now passes").unwrap();
        assert_eq!(RunStrategy::retry_latest(&strategy, &watches), None);

        assert_eq!(
            std::fs::read_to_string(dir.join("fails.log")).unwrap(),
            "fail\nfail\n",
            "the failed job must run exactly twice"
        );
        assert_eq!(
            std::fs::read_to_string(dir.join("passes.log")).unwrap(),
            "pass\n",
            "the passed job must never be retried"
        );

        std::fs::remove_dir_all(&dir).unwrap();
    }

    /// Ctrl-R without any previous run is a clear rejection, never a full run.
    #[test]
    fn keyboard_retry_without_a_previous_blocking_run_is_rejected() {
        use super::BlockingStrategy;

        let dir = scratch("kbd-retry-no-run");
        std::fs::create_dir_all(&dir).unwrap();
        let watches = Arc::new(Mutex::new(Watches::new(vec![rule("my tests")])));
        let strategy = BlockingStrategy::new(dir.clone(), false, false, 1);

        assert_eq!(RunStrategy::retry_latest(&strategy, &watches), None);
        assert!(!dir.join("fails.log").exists());

        std::fs::remove_dir_all(&dir).unwrap();
    }

    /// Ctrl-R after an all-pass run has nothing eligible: rejected, nothing
    /// scheduled, no fallback run.
    #[test]
    fn keyboard_retry_after_a_passing_blocking_run_is_rejected() {
        use super::BlockingStrategy;

        let dir = scratch("kbd-retry-passed");
        std::fs::create_dir_all(&dir).unwrap();
        let passes = Rules::new(
            "passes".to_owned(),
            vec!["echo pass >> passes.log".to_owned()],
            vec!["src/**".to_owned()],
            vec![],
            false,
        );
        let watches = Arc::new(Mutex::new(Watches::new(vec![passes])));
        let strategy = BlockingStrategy::new(dir.clone(), false, false, 1);

        let plan = watches.lock().unwrap().retry_plan(&["passes".to_owned()]);
        strategy.run_manual(plan, None);
        assert_eq!(RunStrategy::retry_latest(&strategy, &watches), None);
        assert_eq!(
            std::fs::read_to_string(dir.join("passes.log")).unwrap(),
            "pass\n",
            "a passing run must never be retried"
        );

        std::fs::remove_dir_all(&dir).unwrap();
    }

    /// Ctrl-R on a socket-enabled non-block watch schedules the retry of the
    /// latest terminal generation and stays idempotent for the same source.
    #[test]
    fn keyboard_retry_schedules_the_latest_terminal_generation() {
        use crate::task_result::{TaskSnapshot, TaskState};

        let watches = Watches::new(vec![rule("my tests")]);
        let worker = Arc::new(workers::Worker::new(false, false, |_| {}));
        let control_state = Arc::new(Mutex::new(WatcherState::default()));
        control_state
            .lock()
            .unwrap()
            .apply(crate::executor::Event::Started {
                run_id: 1,
                trigger: "src/main.rs".to_owned(),
                batch: Some(8),
                predecessor: None,
                changed: vec!["src/main.rs".to_owned()],
                commands: vec!["false".to_owned()],
                target: None,
                execution_signature: None,
                effective_concurrency: None,
                concurrency_source: None,
                revision: None,
                revision_hash: None,
            });
        control_state
            .lock()
            .unwrap()
            .apply(crate::executor::Event::TaskTerminal {
                run_id: 1,
                task: TaskSnapshot {
                    position: 0,
                    id: "my tests".to_owned(),
                    name: "my tests".to_owned(),
                    state: TaskState::Failed,
                    duration_ms: Some(1),
                },
            });
        control_state
            .lock()
            .unwrap()
            .apply(crate::executor::Event::Finished {
                run_id: 1,
                superseded_by: None,
                elapsed: std::time::Duration::from_millis(1),
                failures: vec!["my tests failed".to_owned()],
            });
        let shared_watches = Arc::new(Mutex::new(watches));
        let strategy = NonBlockStrategy::new_arc_with_shared(
            worker.clone(),
            shared_watches.clone(),
            None,
            control_state,
            None,
            None,
            Arc::new(crate::watcher_state::WatcherInstance::new()),
            None,
            None,
            None,
        );
        // The source generation's trigger path is retained by the normal run
        // path; seed it here since the state events were applied manually.
        strategy.remember_filepath_for_test(1, Some("src/main.rs"));

        let first = strategy.retry_latest(&shared_watches);
        assert!(first.is_some(), "a failed latest run must schedule a retry");
        assert_eq!(
            strategy.retry_latest(&shared_watches),
            first,
            "the same latest source must not reschedule"
        );
    }

    /// Ctrl-R on a non-block watch with no runs at all is a rejection.
    #[test]
    fn keyboard_retry_without_a_previous_non_block_run_is_rejected() {
        let watches = Arc::new(Mutex::new(Watches::new(vec![rule("my tests")])));
        let worker = Arc::new(workers::Worker::new(false, false, |_| {}));
        let strategy = NonBlockStrategy::new_arc(
            worker,
            Watches::new(vec![]),
            None,
            Arc::new(Mutex::new(WatcherState::default())),
            None,
            None,
        );

        assert_eq!(strategy.retry_latest(&watches), None);
    }

    /// A committed config change after the latest run rejects the keyboard
    /// retry atomically instead of retrying a partial selection.
    #[test]
    fn keyboard_retry_after_a_blocking_config_change_is_rejected() {
        use super::BlockingStrategy;

        let dir = scratch("kbd-retry-stale");
        std::fs::create_dir_all(&dir).unwrap();
        let fails = Rules::new(
            "fails".to_owned(),
            vec![
                "echo fail >> fails.log; if test -f allow; then exit 0; else exit 1; fi".to_owned(),
            ],
            vec!["src/**".to_owned()],
            vec![],
            false,
        );
        let passes = Rules::new(
            "passes".to_owned(),
            vec!["echo pass >> passes.log".to_owned()],
            vec!["src/**".to_owned()],
            vec![],
            false,
        );
        let watches = Arc::new(Mutex::new(Watches::new(vec![fails, passes.clone()])));
        let strategy = BlockingStrategy::new(dir.clone(), false, false, 1);

        let plan = watches
            .lock()
            .unwrap()
            .retry_plan(&["fails".to_owned(), "passes".to_owned()]);
        strategy.run_manual(plan, None);

        // Live reload removes the failed job; the recorded selection can no
        // longer be reproduced from the committed topology.
        *watches.lock().unwrap() = Watches::new(vec![passes]);

        std::fs::write(dir.join("allow"), "passes now").unwrap();
        assert_eq!(RunStrategy::retry_latest(&strategy, &watches), None);
        assert_eq!(
            std::fs::read_to_string(dir.join("fails.log")).unwrap(),
            "fail\n",
            "a stale selection must not schedule partial work"
        );
        assert_eq!(
            std::fs::read_to_string(dir.join("passes.log")).unwrap(),
            "pass\n",
            "rejection must not schedule any work"
        );

        std::fs::remove_dir_all(&dir).unwrap();
    }
}
