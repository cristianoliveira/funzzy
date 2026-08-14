//! Shared watch orchestration.
//!
//! One application flow owns filesystem readiness and event-to-run
//! conversion. Blocking and cancellable behaviors are injected as
//! [`RunStrategy`] implementations; init and change triggers share one
//! preparation path. CLI commands stay thin: build a strategy and call
//! [`watch_loop`].

use crate::config;
use crate::control::{ControlServer, ControlState, ControlTarget};
use crate::errors::FzzError;
use crate::executor::{Executor, RunMetadata, SystemClock, SystemProcessRunner};
use crate::rules::{self, Rules};
use crate::stdout;
use crate::template;
use crate::watcher;
use crate::watches::Watches;
use crate::workers;
use std::path::PathBuf;
use std::sync::{Arc, Mutex};

/// What the watch loop does once filesystem watches are registered.
pub enum InitAction {
    /// Init-selected rules should run now.
    Run(Vec<Rules>),
    /// Nothing to run on init; wait for file changes.
    Wait,
}

/// One preparation path for init triggers: run when rules exist AND the
/// `run_on_init` flag is enabled, otherwise wait for changes.
pub fn init_action(rules: Option<Vec<Rules>>, run_on_init: bool) -> InitAction {
    match rules {
        Some(rules) if run_on_init => InitAction::Run(rules),
        _ => InitAction::Wait,
    }
}

/// Injected executor strategy: owns how selected rules are executed.
pub trait RunStrategy {
    /// Called once after every filesystem watch is registered, before any
    /// init work, so auxiliary surfaces (e.g. the control socket) publish
    /// only when readiness is truthful. Defaults to no-op.
    fn on_ready(&self) {}

    /// Executes rules selected for init.
    fn run_init(&self, rules: Vec<Rules>);

    /// Executes rules selected for a file change.
    fn run_change(&self, rules: Vec<Rules>, filepath: &str);
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

    watcher::events(
        list_of_watched_paths,
        || {
            strategy.on_ready();

            match init_action(watches.run_on_init(), run_on_init) {
                InitAction::Run(rules) => {
                    stdout::info("Running on init commands.");
                    strategy.run_init(rules);
                }
                InitAction::Wait => stdout::info("Watching..."),
            }
        },
        |file_changed| {
            if let Some(rules) = watches.watch(file_changed) {
                stdout::clear_screen();

                stdout::verbose(
                    &format!("Triggered by change in: {}", file_changed),
                    verbose,
                );

                strategy.run_change(rules, file_changed);
            }
        },
        verbose,
    )
    .map_err(FzzError::GenericError)
}

/// Blocking executor: expands command templates and runs tasks in-process,
/// honoring fail-fast, then presents the results.
pub struct BlockingStrategy {
    root: PathBuf,
    verbose: bool,
    executor: Executor,
}

impl BlockingStrategy {
    pub fn new(root: PathBuf, verbose: bool, fail_fast: bool) -> Self {
        let executor = Executor::new(
            Arc::new(SystemProcessRunner),
            Arc::new(SystemClock),
            1,
            Arc::new(|_| {}),
            fail_fast,
            verbose,
        )
        .expect("concurrency one is supported");
        BlockingStrategy {
            root,
            verbose,
            executor,
        }
    }

    fn expand(&self, rules: Vec<Rules>, filepath: Option<&str>) -> Vec<rules::CommandLine> {
        rules::command_lines(rules)
            .into_iter()
            .map(|command| {
                let expanded = template::template_line(
                    command,
                    template::TemplateOptions {
                        filepath: filepath.map(str::to_string),
                        current_dir: format!("{}", self.root.display()),
                    },
                );
                for variable in &expanded.unknown_variables {
                    stdout::warn(&format!("Unknown template variable '{}'.", variable));
                }
                expanded.command
            })
            .collect()
    }

    fn execute_tasks(
        &self,
        metadata: RunMetadata,
        tasks: Vec<rules::CommandLine>,
    ) -> crate::executor::CompletedRun {
        self.executor.run_to_completion(metadata, tasks)
    }
}

impl RunStrategy for BlockingStrategy {
    fn run_init(&self, rules: Vec<Rules>) {
        let tasks = self.expand(rules, None);
        let completed = self.execute_tasks(RunMetadata::new(0, "init"), tasks);
        stdout::present_results(completed.results, completed.elapsed);
    }

    fn run_change(&self, rules: Vec<Rules>, filepath: &str) {
        stdout::verbose(&format!("Rules: {:?}", rules), self.verbose);
        stdout::verbose(
            &format!("Formatted rules:\n{}", config::format_rules(&rules)),
            self.verbose,
        );

        let tasks = self.expand(rules, Some(filepath));
        let completed = self.execute_tasks(RunMetadata::new(0, filepath), tasks);
        stdout::present_results(completed.results, completed.elapsed);
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
    ) -> Arc<Self> {
        let strategy = Arc::new(NonBlockStrategy {
            worker,
            watches,
            control_socket,
            control_state,
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

        match ControlServer::start_with_runner(
            path,
            Arc::clone(&self.control_state),
            targets,
            move |target| runner.run_target(&target),
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

    /// Runs a control-requested target through the worker run contract:
    /// cancel the active run, then schedule the target's rules.
    pub fn run_target(&self, target: &str) -> Result<u64, String> {
        let rules = self
            .watches
            .target(target)
            .ok_or_else(|| format!("No target found for '{}'", target))?;
        self.worker.cancel_running_tasks()?;
        self.worker
            .schedule_with_trigger(rules, &format!("control:{}", target), None)
    }
}

impl RunStrategy for NonBlockStrategy {
    fn on_ready(&self) {
        if let Some(server) = self.start_control_server() {
            *self.control_server.lock().unwrap() = Some(server);
        }
    }

    fn run_init(&self, rules: Vec<Rules>) {
        if let Err(err) = self.worker.schedule(rules, "") {
            stdout::error(&format!("failed to initiate next run: {:?}", err));
        }
    }

    fn run_change(&self, rules: Vec<Rules>, filepath: &str) {
        if let Err(err) = self.worker.cancel_running_tasks() {
            stdout::error(&format!(
                "failed to cancel current running tasks: {:?}",
                err
            ));
        }

        if let Err(err) = self.worker.schedule(rules, filepath) {
            stdout::error(&format!("failed to initiate next run: {:?}", err));
        }
    }
}

#[cfg(test)]
mod tests {
    use super::init_action;
    use super::InitAction;
    use super::NonBlockStrategy;
    use crate::control::{ControlServer, ControlState};
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

    #[test]
    fn it_runs_init_rules_when_flag_is_enabled_and_rules_exist() {
        assert!(matches!(
            init_action(Some(vec![rule("build")]), true),
            InitAction::Run(_)
        ));
    }

    #[test]
    fn it_waits_when_run_on_init_flag_is_disabled() {
        assert!(matches!(
            init_action(Some(vec![rule("build")]), false),
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
        let strategy = NonBlockStrategy::new_arc(worker, watches, None, control_state);

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
        let strategy = NonBlockStrategy::new_arc(worker, watches, None, control_state);

        let err = strategy.run_target("nope").expect_err("unknown target");
        assert!(
            err.contains("No target found for 'nope'"),
            "unexpected: {}",
            err
        );
    }
}
