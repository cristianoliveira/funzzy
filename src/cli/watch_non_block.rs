use crate::awaiting::AwaitCoordinator;
use crate::cli::Command;
use crate::control::{ControlInstance, ControlState};
use crate::duration_recorder::DurationRecorder;
use crate::duration_store::{state_file_path, DurationStore, STATE_SCHEMA_VERSION};
use crate::errors::FzzError;
use crate::output::OutputRegistry;
use crate::snapshot::SnapshotBroker;
use crate::watch_loop::{watch_loop, NonBlockStrategy};
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
    reload_ready: Option<std::sync::Arc<std::sync::mpsc::Receiver<()>>>,
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
        self.reload_ready = Some(std::sync::Arc::new(ready));
        self
    }
}

impl Command for WatchNonBlockCommand {
    fn execute(&self) -> Result<(), FzzError> {
        let control_state = Arc::new(Mutex::new(ControlState::default()));
        let coordinator = Arc::new(AwaitCoordinator::new());
        let outputs = Arc::new(OutputRegistry::new());
        let instance = Arc::new(ControlInstance::new());
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
                    coordinator_state.observe(&event);
                    if let Some(stream) = &events_state {
                        stream.emit_event(event.clone());
                    }
                    worker_state.lock().unwrap().apply(event);
                    broker_state.publish();
                },
                Some(worker_outputs),
            )
            .with_hooks(hooks)
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

        let strategy = NonBlockStrategy::new_arc_with_subscription(
            worker,
            self.watches.clone(),
            self.control_socket.clone(),
            control_state,
            Some(coordinator),
            Some(outputs),
            instance,
            Some(broker),
            Some(recorder),
        );
        watch_loop(
            &shared,
            self.run_on_init,
            &*strategy,
            self.watches.debounce(),
            self.verbose,
            Some(swap_rx),
            self.reload_ready.clone(),
        )
    }
}
