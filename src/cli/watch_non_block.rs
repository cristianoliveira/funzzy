use crate::awaiting::AwaitCoordinator;
use crate::cli::Command;
use crate::duration_recorder::DurationRecorder;
use crate::duration_store::{state_file_path, DurationStore, STATE_SCHEMA_VERSION};
use crate::errors::FzzError;
use crate::output::OutputRegistry;
use crate::snapshot::SnapshotBroker;
use crate::watch_loop::{watch_loop, NonBlockStrategy};
use crate::watcher_state::{WatcherInstance, WatcherState};
use crate::watches::Watches;
use crate::workers;
use std::path::PathBuf;
use std::sync::{Arc, Mutex};

/// # `WatchNonBlockCommand`
///
/// Starts watcher to listen the change events configured
/// in watch.yaml, upon change it cancell all previous tasks, including the running
/// task and initiate a new set of tasks.
///
pub struct WatchNonBlockCommand {
    watches: Watches,
    verbose: bool,
    fail_fast: bool,
    run_on_init: bool,
    control_socket: Option<PathBuf>,
    /// NDJSON run-event stream destination (TASK-0039); None = no stream.
    events: Option<Arc<crate::event_stream::EventStream>>,
    /// In-process reload coordinator (TASK-0090); None for legacy callers.
    reload: Option<crate::reload_coordinator::ReloadCoordinator>,
    /// Reload-watcher readiness signal (TASK-0090); init waits for it.
    reload_ready: Option<std::sync::Arc<std::sync::Mutex<std::sync::mpsc::Receiver<()>>>>,
    /// Shared watcher shutdown coordinator (TASK-0101).
    shutdown: Option<Arc<crate::shutdown::ShutdownCoordinator>>,
}

impl WatchNonBlockCommand {
    pub fn new(
        watches: Watches,
        verbose: bool,
        fail_fast: bool,
        run_on_init: bool,
        control_socket: Option<PathBuf>,
    ) -> Self {
        Self::with_events(
            watches,
            verbose,
            fail_fast,
            run_on_init,
            control_socket,
            None,
        )
    }

    /// Creates the non-block watch command with an optional NDJSON run-event
    /// stream (TASK-0039).
    pub fn with_events(
        watches: Watches,
        verbose: bool,
        fail_fast: bool,
        run_on_init: bool,
        control_socket: Option<PathBuf>,
        events: Option<Arc<crate::event_stream::EventStream>>,
    ) -> Self {
        WatchNonBlockCommand {
            watches,
            verbose,
            fail_fast,
            run_on_init,
            control_socket,
            events,
            reload: None,
            reload_ready: None,
            shutdown: None,
        }
    }

    /// Attaches the in-process reload coordinator (TASK-0090).
    pub fn with_reload(mut self, reload: crate::reload_coordinator::ReloadCoordinator) -> Self {
        self.reload = Some(reload);
        self
    }

    /// Attaches the reload-watcher readiness signal; init waits for it
    /// before running (TASK-0090).
    pub fn with_reload_ready(mut self, ready: std::sync::mpsc::Receiver<()>) -> Self {
        self.reload_ready = Some(std::sync::Arc::new(std::sync::Mutex::new(ready)));
        self
    }

    pub fn with_shutdown(mut self, shutdown: Arc<crate::shutdown::ShutdownCoordinator>) -> Self {
        self.shutdown = Some(shutdown);
        self
    }
}

impl Command for WatchNonBlockCommand {
    fn execute(&self) -> Result<(), FzzError> {
        let control_state = Arc::new(Mutex::new(WatcherState::default()));
        let coordinator = Arc::new(AwaitCoordinator::new());
        let outputs = Arc::new(OutputRegistry::new());
        let instance = Arc::new(WatcherInstance::new());
        // Duration recorder (TASK-0054): control-run targets record terminal
        // wall durations against their execution signature; fs/init/emit runs
        // carry no signature and are ignored by the recorder. The same
        // recorder drives target estimates and run-start snapshot estimates
        // (TASK-0055).
        let recorder = Arc::new(DurationRecorder::new(DurationStore::new(state_file_path(
            &std::fs::canonicalize(self.watches.root())
                .unwrap_or_else(|_| self.watches.root().to_path_buf()),
            STATE_SCHEMA_VERSION,
        ))));
        // Frozen run-start estimate lookup for correlated snapshots
        // (TASK-0055): estimates the current generation's target history at
        // run start; None for non-target generations.
        let recorder_lookup = Arc::clone(&recorder);
        let snapshot_estimates: crate::snapshot::EstimateLookup =
            Arc::new(move |run_id| recorder_lookup.estimate_at_start(run_id));
        let broker = Arc::new(SnapshotBroker::with_outputs(
            instance.as_ref().clone(),
            Arc::clone(&control_state),
            Arc::clone(&coordinator),
            Some(Arc::clone(&outputs)),
            Some(snapshot_estimates),
            self.watches.concurrency(),
        ));
        let worker_state = Arc::clone(&control_state);
        let coordinator_state = Arc::clone(&coordinator);
        let worker_outputs = Arc::clone(&outputs);
        let broker_state = Arc::clone(&broker);
        let recorder_state = Arc::clone(&recorder);
        let events_state = self.events.clone();
        let hooks = self.watches.hooks();
        let worker = Arc::new(
            workers::Worker::with_root_and_concurrency_and_outputs(
                self.verbose,
                self.fail_fast,
                self.watches.root().to_path_buf(),
                self.watches.concurrency(),
                move |event| {
                    recorder_state.observe(&event);
                    // Apply the correlated snapshot before waking exact
                    // awaiters. Otherwise a terminal coordinator transition
                    // can race its snapshot projection and return a failed
                    // reason with a still-running snapshot.
                    worker_state.lock().unwrap().apply(event.clone());
                    coordinator_state.observe(&event);
                    if let Some(stream) = &events_state {
                        stream.emit_event(event);
                    }
                    broker_state.publish();
                },
                Some(worker_outputs),
            )
            .with_hooks(hooks)
            .with_recovery_policy(self.watches.recovery_policy())
            .with_recovery_timeout(self.watches.recovery_timeout())
            .with_revision(self.watches.revision().cloned().unwrap_or(
                crate::config_revision::ConfigRevision {
                    number: 0,
                    hash: String::new(),
                },
            )),
        );

        // TASK-0090: the shared watch config the routing loop reads per batch
        // and the reload coordinator swaps at the commit boundary. Install
        // the worker + root-swap publisher BEFORE the strategy takes
        // ownership, so a reload commit can reach them.
        let (swap_tx, swap_rx) = std::sync::mpsc::channel();
        let shared = match &self.reload {
            Some(reload) => std::sync::Arc::clone(reload.shared()),
            None => std::sync::Arc::new(std::sync::Mutex::new(self.watches.clone())),
        };
        if let Some(reload) = &self.reload {
            reload.install_worker(Arc::clone(&worker));
            reload.install_publisher(crate::watcher::RootSwapPublisher::new(swap_tx));
        }

        // TASK-0091 AC3: the config lifecycle source is shared by the reload
        // thread (writer) and the control/broker surfaces (readers).
        let lifecycle = self
            .reload
            .as_ref()
            .map(|reload| Arc::clone(reload.lifecycle()));
        let strategy = NonBlockStrategy::new_arc_with_shared(
            worker,
            Arc::clone(&shared),
            self.control_socket.clone(),
            control_state,
            Some(coordinator),
            Some(outputs),
            instance,
            Some(Arc::clone(&broker)),
            Some(recorder),
            lifecycle,
        );
        // TASK-0090 AC8: when the reloaded config changes the control socket
        // path, the transaction binds the new socket before commit and
        // retires the old after — through the strategy that owns the live
        // server. A bind failure surfaces as a prepare error (fatal path).
        if let Some(reload) = &self.reload {
            let swapper_strategy = Arc::clone(&strategy);
            let retire_strategy = Arc::clone(&strategy);
            reload.install_socket_swapper(crate::reload_coordinator::SocketSwapper::new(
                move |path| swapper_strategy.prepare_socket_swap(path.to_path_buf()),
                move || retire_strategy.retire_socket_swap(),
            ));
            // TASK-0091 AC3/AC4: the snapshot broker carries the config
            // lifecycle transition and publishes on every lifecycle change,
            // so active subscriptions receive configReloading/configReloaded/
            // configInvalid without disconnecting or polling.
            broker.attach_lifecycle(Arc::clone(reload.lifecycle()));
            let broker_pub = Arc::clone(&broker);
            reload
                .lifecycle()
                .watch(Arc::new(move |_| broker_pub.publish()));
        }
        watch_loop(
            &shared,
            self.run_on_init,
            &*strategy,
            self.watches.debounce(),
            self.verbose,
            Some(swap_rx),
            self.reload_ready.clone(),
            self.shutdown.clone(),
        )
    }
}
