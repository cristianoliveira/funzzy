//! Shared watch orchestration.
//!
//! One application flow owns filesystem readiness and event-to-run
//! conversion. Blocking and cancellable behaviors are injected as
//! [`RunStrategy`] implementations; init and change triggers share one
//! preparation path. CLI commands stay thin: build a strategy and call
//! [`watch_loop`].

use crate::awaiting::AwaitCoordinator;
use crate::control::{
    ControlInstance, ControlServer, ControlState, ControlTarget, EmitOutcome,
    TargetEstimateProvider,
};
use crate::diagnostics;
use crate::duration_recorder::DurationRecorder;
use crate::errors::FzzError;
use crate::executor::RunMetadata;
use crate::identity::{Batch, BatchId};
use crate::output::OutputRegistry;
use crate::plan::RunPlan;
use crate::snapshot::SnapshotBroker;
use crate::stdout;
use crate::watcher::{self, FileEvent};
use crate::watches::Watches;
use crate::workers;
use crate::workflow::WorkflowRunner;
use std::path::PathBuf;
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
    /// Called once after every filesystem watch is registered, before any
    /// init work, so auxiliary surfaces (e.g. the control socket) publish
    /// only when readiness is truthful. Defaults to no-op.
    fn on_ready(&self) {}

    /// Executes plan selected for init. Returns the scheduled generation when
    /// the strategy schedules asynchronous work (restart policy); blocking
    /// strategies run in-process and return `None`.
    fn run_init(&self, plan: RunPlan) -> Option<u64>;

    /// The busy-run policy this strategy implements: `restart` replaces the
    /// active run with newer work; `wait` runs in-process and blocks.
    fn policy(&self) -> &'static str;

    /// Executes plan selected for a file change. The batch carries the
    /// debounce identity and complete changed-path set (contract §1); the
    /// trigger path is the deterministic first match. Returns the scheduled
    /// generation for non-blocking strategies, `None` for in-process runs.
    fn run_change(&self, plan: RunPlan, filepath: &str, batch: &Batch) -> Option<u64>;

    /// Called with each normalized batch before routing (default no-op), so
    /// the pending-debounce observation reflects open windows.
    fn on_batch(&self, _batch: &Batch) {}

    /// Called after the batch finished routing (scheduled or explicit no-op).
    fn on_batch_complete(&self) {}
}

/// Runs the watch loop: registers filesystem watches, publishes readiness,
/// converts init and change events into rule selections, and delegates
/// execution to the injected strategy.
pub fn watch_loop(
    watches: &Watches,
    run_on_init: bool,
    strategy: &dyn RunStrategy,
    debounce: Duration,
    verbose: bool,
) -> Result<(), FzzError> {
    let list_of_watched_paths = watches.paths_to_watch().unwrap_or_default();

    watcher::events(
        list_of_watched_paths,
        || {
            strategy.on_ready();

            match init_action(watches.run_on_init_plan(), run_on_init) {
                InitAction::Run(plan) => {
                    stdout::info("Running on init commands.");
                    let commands = plan.commands().len();
                    let generation = strategy.run_init(plan);
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
            match watches.watch_plan_batch(&batch.changed) {
                Some((plan, trigger)) => {
                    stdout::clear_screen();
                    if verbose {
                        emit_matched_decisions(watches, &batch, &trigger);
                    }
                    let generation = strategy.run_change(plan, &trigger, &batch);
                    if verbose {
                        observe_triggers(watches, &batch, &trigger, generation);
                    }
                }
                None => {
                    if verbose {
                        emit_non_matched_decisions(watches, &batch);
                    }
                }
            }
            strategy.on_batch_complete();
        },
        debounce,
        verbose,
    )
    .map_err(FzzError::GenericError)
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
}

impl BlockingStrategy {
    pub fn new(root: PathBuf, verbose: bool, fail_fast: bool, concurrency: usize) -> Self {
        Self {
            workflow: WorkflowRunner::new(root, verbose, fail_fast, concurrency),
        }
    }
}

impl RunStrategy for BlockingStrategy {
    fn policy(&self) -> &'static str {
        "wait"
    }

    fn run_init(&self, plan: RunPlan) -> Option<u64> {
        match self.workflow.run(plan, RunMetadata::new(0, "init"), None) {
            Ok(completed) => stdout::present_results(
                completed.results,
                completed.elapsed,
                Some(&completed.outcome),
            ),
            Err(error) => stdout::error(&error),
        }
        None
    }

    fn run_change(&self, plan: RunPlan, filepath: &str, batch: &Batch) -> Option<u64> {
        let metadata =
            RunMetadata::correlated(0, filepath, Some(batch.id.0), None, batch.changed.clone());
        match self.workflow.run(plan, metadata, Some(filepath)) {
            Ok(completed) => stdout::present_results(
                completed.results,
                completed.elapsed,
                Some(&completed.outcome),
            ),
            Err(error) => stdout::error(&error),
        }
        None
    }
}

/// Cancellable executor: schedules runs on the worker, cancelling any active
/// run before replacement work, and publishes the control surface after
/// readiness.
pub struct NonBlockStrategy {
    worker: Arc<workers::Worker>,
    watches: Watches,
    control_socket: Option<PathBuf>,
    control_state: Arc<Mutex<ControlState>>,
    coordinator: Option<Arc<AwaitCoordinator>>,
    outputs: Option<Arc<OutputRegistry>>,
    instance: Arc<ControlInstance>,
    broker: Option<Arc<SnapshotBroker>>,
    control_server: Mutex<Option<ControlServer>>,
    self_arc: Mutex<Option<Arc<NonBlockStrategy>>>,
    /// Optional duration recorder (TASK-0055): wires the estimate provider
    /// into `targets`, capabilities, and correlated snapshots.
    recorder: Option<Arc<DurationRecorder>>,
}

impl NonBlockStrategy {
    /// Creates the strategy inside an `Arc`; the arc lets the control server
    /// call back through the run orchestration contract.
    pub fn new_arc(
        worker: Arc<workers::Worker>,
        watches: Watches,
        control_socket: Option<PathBuf>,
        control_state: Arc<Mutex<ControlState>>,
        coordinator: Option<Arc<AwaitCoordinator>>,
        outputs: Option<Arc<OutputRegistry>>,
    ) -> Arc<Self> {
        Self::new_arc_with_subscription(
            worker,
            watches,
            control_socket,
            control_state,
            coordinator,
            outputs,
            Arc::new(ControlInstance::new()),
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
        control_state: Arc<Mutex<ControlState>>,
        coordinator: Option<Arc<AwaitCoordinator>>,
        outputs: Option<Arc<OutputRegistry>>,
        instance: Arc<ControlInstance>,
        broker: Option<Arc<SnapshotBroker>>,
        recorder: Option<Arc<DurationRecorder>>,
    ) -> Arc<Self> {
        let strategy = Arc::new(NonBlockStrategy {
            worker,
            watches,
            control_socket,
            control_state,
            coordinator,
            outputs,
            instance,
            broker,
            control_server: Mutex::new(None),
            self_arc: Mutex::new(None),
            recorder,
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
        let runner = self
            .self_arc
            .lock()
            .unwrap()
            .clone()
            .expect("self arc set by new_arc");
        let targets = self
            .watches
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
        let emit_runner = Arc::clone(&runner);
        let cancel_runner = Arc::clone(&runner);
        let coordinator = self.coordinator.clone();
        let outputs = self.outputs.clone();
        let instance = Arc::clone(&self.instance);
        let broker = self.broker.clone();
        // TASK-0055: estimate provider computed at request time from the
        // target's resolved plan signature; None when no recorder is wired.
        let estimates: Option<TargetEstimateProvider> = self.recorder.as_ref().map(|recorder| {
            let watches = self.watches.clone();
            let recorder = Arc::clone(recorder);
            let concurrency = self.worker.concurrency();
            let fail_fast = self.worker.fail_fast();
            let root = self.watches.root().to_path_buf();
            let provider: TargetEstimateProvider = Arc::new(move |target: &ControlTarget| {
                let Some(plan) = watches.target_plan(&target.name) else {
                    return None;
                };
                let Ok(plan) = plan.resolve_context(&root) else {
                    return None;
                };
                let signature = plan.execution_signature(concurrency, fail_fast);
                recorder.estimate(&signature, None)
            });
            provider
        });
        let start = if let Some(broker) = broker {
            match estimates {
                Some(estimates) => ControlServer::start_with_broker_and_estimates(
                    path,
                    Arc::clone(&self.control_state),
                    targets,
                    move |target, sequential| run_runner.run_target(&target, sequential),
                    move |path| emit_runner.emit_path(&path),
                    coordinator,
                    outputs,
                    move |generation| cancel_runner.cancel_generation(generation),
                    instance,
                    broker,
                    estimates,
                ),
                None => ControlServer::start_with_broker(
                    path,
                    Arc::clone(&self.control_state),
                    targets,
                    move |target, sequential| run_runner.run_target(&target, sequential),
                    move |path| emit_runner.emit_path(&path),
                    coordinator,
                    outputs,
                    move |generation| cancel_runner.cancel_generation(generation),
                    instance,
                    broker,
                ),
            }
        } else {
            ControlServer::start_with_cancel(
                path,
                Arc::clone(&self.control_state),
                targets,
                move |target, sequential| run_runner.run_target(&target, sequential),
                move |path| emit_runner.emit_path(&path),
                coordinator,
                outputs,
                move |generation| cancel_runner.cancel_generation(generation),
            )
        };
        match start {
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

    /// Cancels an exact generation through the worker run contract
    /// (TASK-0046): a compare-and-act on generation identity. Returns the
    /// disposition or a no-op for already-terminal/unknown generations.
    pub fn cancel_generation(
        &self,
        generation: u64,
    ) -> Result<crate::workers::CancelResult, String> {
        self.worker.cancel_generation(generation)
    }

    /// Runs a control-requested target through the worker run contract:
    /// cancel the active run, then schedule the target's rules with its
    /// structural target identity and execution signature (TASK-0054).
    /// `sequential` (TASK-0073) requests effective concurrency one for this
    /// exact generation only; later native generations keep their configured
    /// concurrency.
    pub fn run_target(&self, target: &str, sequential: bool) -> Result<u64, String> {
        let plan = self
            .watches
            .target_plan(target)
            .ok_or_else(|| format!("No target found for '{}'", target))?;
        let commands = plan.commands().len();
        self.worker.cancel_running_tasks()?;
        let run_id = self.worker.schedule_target(plan, target, sequential)?;
        diagnostics::debug(&diagnostics::Record {
            source: Some("control"),
            decision: Some("scheduled"),
            generation: Some(run_id),
            policy: Some("restart"),
            commands: Some(commands),
            task: Some(target.to_owned()),
            ..Default::default()
        });
        Ok(run_id)
    }

    /// Routes one synthetic path change through the exact shared policy used
    /// for native filesystem events: `watch_plan` (normalization, change
    /// match, ignore precedence, task ordering, `run_on_init` exclusions),
    /// then the same cancel-and-schedule busy-run contract. Unmatched and
    /// ignored paths are explicit outcomes with no scheduled generation.
    pub fn emit_path(&self, path: &str) -> Result<EmitOutcome, String> {
        match self.watches.watch_plan(path) {
            Some(plan) => {
                let matched = plan.task_names();
                let commands = plan.commands().len();
                self.worker.cancel_running_tasks()?;
                let run_id = self.worker.schedule_plan(plan, path)?;
                diagnostics::debug(&diagnostics::Record {
                    source: Some("control"),
                    decision: Some("scheduled"),
                    generation: Some(run_id),
                    policy: Some("restart"),
                    commands: Some(commands),
                    path: Some(path.to_owned()),
                    normalized: Some(self.watches.normalized_path(path)),
                    ..Default::default()
                });
                Ok(EmitOutcome::scheduled(matched, run_id))
            }
            None => {
                let explained = self.watches.explain(path);
                diagnostics::debug(&diagnostics::Record {
                    source: Some("control"),
                    decision: Some(if explained.ignored.is_empty() {
                        "unmatched"
                    } else {
                        "ignored"
                    }),
                    path: Some(path.to_owned()),
                    normalized: Some(self.watches.normalized_path(path)),
                    ..Default::default()
                });
                Ok(if explained.ignored.is_empty() {
                    EmitOutcome::unmatched()
                } else {
                    EmitOutcome::ignored()
                })
            }
        }
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

    fn run_init(&self, plan: RunPlan) -> Option<u64> {
        match self.worker.schedule_plan(plan, "") {
            Ok(run_id) => Some(run_id),
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

    fn run_change(&self, plan: RunPlan, filepath: &str, batch: &Batch) -> Option<u64> {
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
        ) {
            Ok(run_id) => {
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
}

#[cfg(test)]
mod tests {
    use super::init_action;
    use super::InitAction;
    use super::NonBlockStrategy;
    use super::RunStrategy;
    use crate::control::{ControlServer, ControlState};
    use crate::plan::RunPlan;
    use crate::rules::Rules;
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
        let holder_state = Arc::new(Mutex::new(ControlState::default()));
        let _holder = ControlServer::start(&path, holder_state).unwrap();

        let worker = Arc::new(workers::Worker::new(false, false, |_| {}));
        let control_state = Arc::new(Mutex::new(ControlState::default()));
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
        let control_state = Arc::new(Mutex::new(ControlState::default()));
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
        let control_state = Arc::new(Mutex::new(ControlState::default()));
        let strategy = NonBlockStrategy::new_arc(worker, watches, None, control_state, None, None);

        let run_id = strategy
            .run_target("my tests", false)
            .expect("known target should schedule");
        assert_eq!(run_id, 1);
    }

    #[test]
    fn it_rejects_unknown_control_targets() {
        let watches = Watches::new(vec![rule("my tests")]);
        let worker = Arc::new(workers::Worker::new(false, false, |_| {}));
        let control_state = Arc::new(Mutex::new(ControlState::default()));
        let strategy = NonBlockStrategy::new_arc(worker, watches, None, control_state, None, None);

        let err = strategy
            .run_target("nope", false)
            .expect_err("unknown target");
        assert!(
            err.contains("No target found for 'nope'"),
            "unexpected: {}",
            err
        );
    }

    #[test]
    fn it_emits_a_path_through_the_worker_contract() {
        let watches = Watches::new(vec![rule("my tests")]);
        let worker = Arc::new(workers::Worker::new(false, false, |_| {}));
        let control_state = Arc::new(Mutex::new(ControlState::default()));
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
        let control_state = Arc::new(Mutex::new(ControlState::default()));
        let strategy = NonBlockStrategy::new_arc(worker, watches, None, control_state, None, None);

        let relative = strategy.emit_path("src/main.rs").expect("relative");
        let absolute = strategy
            .emit_path(&format!(
                "{}/src/main.rs",
                strategy.watches.root().display()
            ))
            .expect("absolute");
        assert_eq!(relative.outcome, "scheduled");
        assert_eq!(absolute.outcome, "scheduled");
        assert_eq!(absolute.matched, vec!["my tests".to_owned()]);
    }

    #[test]
    fn it_reports_unmatched_emit_without_scheduling() {
        let watches = Watches::new(vec![rule("my tests")]);
        let worker = Arc::new(workers::Worker::new(false, false, |_| {}));
        let control_state = Arc::new(Mutex::new(ControlState::default()));
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
        let control_state = Arc::new(Mutex::new(ControlState::default()));
        let strategy = NonBlockStrategy::new_arc(worker, watches, None, control_state, None, None);

        let outcome = strategy
            .emit_path("src/generated/out.rs")
            .expect("ignored must not fail");
        assert_eq!(outcome.outcome, "ignored");
        assert!(outcome.matched.is_empty());
        assert_eq!(outcome.run_id, None);
    }

    #[test]
    fn it_marks_pending_debounce_through_the_coordinator() {
        use crate::awaiting::AwaitCoordinator;
        use crate::identity::Batch;
        use crate::output::OutputRegistry;

        let watches = Watches::new(vec![rule("my tests")]);
        let worker = Arc::new(workers::Worker::new(false, false, |_| {}));
        let control_state = Arc::new(Mutex::new(ControlState::default()));
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
            &Arc::new(Mutex::new(ControlState::default())),
            None,
            None,
        );
        assert_eq!(result.latest_batch, Some(9));
        assert!(!result.pending_work.debounce_active);
    }

    #[test]
    fn it_does_not_schedule_when_only_ignored_rules_exist() {
        let watches = Watches::new(vec![rule_with_ignore("my tests")]);
        let worker = Arc::new(workers::Worker::new(false, false, |_| {}));
        let control_state = Arc::new(Mutex::new(ControlState::default()));
        let strategy = NonBlockStrategy::new_arc(worker, watches, None, control_state, None, None);

        let outcome = strategy
            .emit_path("src/generated/out.rs")
            .expect("ignored path");
        assert_eq!(outcome.run_id, None);
    }
}
