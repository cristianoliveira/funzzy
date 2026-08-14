//! Shared watch orchestration.
//!
//! One application flow owns filesystem readiness and event-to-run
//! conversion. Blocking and cancellable behaviors are injected as
//! [`RunStrategy`] implementations; init and change triggers share one
//! preparation path. CLI commands stay thin: build a strategy and call
//! [`watch_loop`].

use crate::awaiting::AwaitCoordinator;
use crate::control::{ControlServer, ControlState, ControlTarget, EmitOutcome};
use crate::errors::FzzError;
use crate::executor::RunMetadata;
use crate::identity::{AtomicSequence, Batch, BatchId};
use crate::output::OutputRegistry;
use crate::plan::RunPlan;
use crate::stdout;
use crate::watcher;
use crate::watches::Watches;
use crate::workers;
use crate::workflow::WorkflowRunner;
use std::path::PathBuf;
use std::sync::{Arc, Mutex};

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

    /// Executes plan selected for init.
    fn run_init(&self, plan: RunPlan);

    /// Executes plan selected for a file change. The batch carries the
    /// debounce identity and complete changed-path set (contract §1); the
    /// trigger path is the deterministic first match.
    fn run_change(&self, plan: RunPlan, filepath: &str, batch: &Batch);

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
    verbose: bool,
) -> Result<(), FzzError> {
    let list_of_watched_paths = watches.paths_to_watch().unwrap_or_default();
    let batch_sequence = AtomicSequence::new();

    watcher::events(
        list_of_watched_paths,
        || {
            strategy.on_ready();

            match init_action(watches.run_on_init_plan(), run_on_init) {
                InitAction::Run(plan) => {
                    stdout::info("Running on init commands.");
                    strategy.run_init(plan);
                }
                InitAction::Wait => stdout::info("Watching..."),
            }
        },
        |changed_paths: &[String]| {
            // One debounce window is one normalized event batch: deduplicated
            // and deterministically ordered, mapped to zero or one generation.
            let batch = Batch::normalized(BatchId(batch_sequence.next()), changed_paths.to_vec());
            if batch.is_empty() {
                return;
            }
            strategy.on_batch(&batch);
            if let Some((plan, trigger)) = watches.watch_plan_batch(&batch.changed) {
                stdout::clear_screen();

                stdout::verbose(&format!("Triggered by change in: {}", trigger), verbose);

                strategy.run_change(plan, &trigger, &batch);
            }
            strategy.on_batch_complete();
        },
        verbose,
    )
    .map_err(FzzError::GenericError)
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
    fn run_init(&self, plan: RunPlan) {
        match self.workflow.run(plan, RunMetadata::new(0, "init"), None) {
            Ok(completed) => stdout::present_results(completed.results, completed.elapsed),
            Err(error) => stdout::error(&error),
        }
    }

    fn run_change(&self, plan: RunPlan, filepath: &str, batch: &Batch) {
        let metadata =
            RunMetadata::correlated(0, filepath, Some(batch.id.0), None, batch.changed.clone());
        match self.workflow.run(plan, metadata, Some(filepath)) {
            Ok(completed) => stdout::present_results(completed.results, completed.elapsed),
            Err(error) => stdout::error(&error),
        }
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
    control_server: Mutex<Option<ControlServer>>,
    self_arc: Mutex<Option<Arc<NonBlockStrategy>>>,
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
        let strategy = Arc::new(NonBlockStrategy {
            worker,
            watches,
            control_socket,
            control_state,
            coordinator,
            outputs,
            control_server: Mutex::new(None),
            self_arc: Mutex::new(None),
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
        match ControlServer::start_with_cancel(
            path,
            Arc::clone(&self.control_state),
            targets,
            move |target| run_runner.run_target(&target),
            move |path| emit_runner.emit_path(&path),
            coordinator,
            outputs,
            move |generation| cancel_runner.cancel_generation(generation),
        ) {
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
    /// cancel the active run, then schedule the target's rules.
    pub fn run_target(&self, target: &str) -> Result<u64, String> {
        let plan = self
            .watches
            .target_plan(target)
            .ok_or_else(|| format!("No target found for '{}'", target))?;
        self.worker.cancel_running_tasks()?;
        self.worker
            .schedule_plan_with_trigger(plan, &format!("control:{}", target), None)
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
                self.worker.cancel_running_tasks()?;
                let run_id = self.worker.schedule_plan(plan, path)?;
                Ok(EmitOutcome::scheduled(matched, run_id))
            }
            None => {
                let explained = self.watches.explain(path);
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
    fn on_ready(&self) {
        if let Some(server) = self.start_control_server() {
            *self.control_server.lock().unwrap() = Some(server);
        }
    }

    fn run_init(&self, plan: RunPlan) {
        if let Err(err) = self.worker.schedule_plan(plan, "") {
            stdout::error(&format!("failed to initiate next run: {:?}", err));
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

    fn run_change(&self, plan: RunPlan, filepath: &str, batch: &Batch) {
        if let Err(err) = self.worker.cancel_running_tasks() {
            stdout::error(&format!(
                "failed to cancel current running tasks: {:?}",
                err
            ));
        }

        if let Err(err) = self.worker.schedule_plan_correlated(
            plan,
            filepath,
            Some(filepath),
            Some(batch.id.0),
            batch.changed.clone(),
        ) {
            stdout::error(&format!("failed to initiate next run: {:?}", err));
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
            .run_target("my tests")
            .expect("known target should schedule");
        assert_eq!(run_id, 1);
    }

    #[test]
    fn it_rejects_unknown_control_targets() {
        let watches = Watches::new(vec![rule("my tests")]);
        let worker = Arc::new(workers::Worker::new(false, false, |_| {}));
        let control_state = Arc::new(Mutex::new(ControlState::default()));
        let strategy = NonBlockStrategy::new_arc(worker, watches, None, control_state, None, None);

        let err = strategy.run_target("nope").expect_err("unknown target");
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
