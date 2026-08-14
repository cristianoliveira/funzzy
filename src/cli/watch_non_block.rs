use crate::awaiting::AwaitCoordinator;
use crate::cli::Command;
use crate::control::{ControlInstance, ControlState};
use crate::duration_recorder::DurationRecorder;
use crate::duration_store::{state_file_path, DurationStore, STATE_SCHEMA_VERSION};
use crate::errors::FzzError;
use crate::output::OutputRegistry;
use crate::snapshot::SnapshotBroker;
use crate::stdout;
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
}

impl WatchNonBlockCommand {
    pub fn new(
        watches: Watches,
        verbose: bool,
        fail_fast: bool,
        run_on_init: bool,
        control_socket: Option<PathBuf>,
    ) -> Self {
        stdout::verbose(&watches.diagnostic_summary(), verbose);

        WatchNonBlockCommand {
            watches,
            verbose,
            fail_fast,
            run_on_init,
            control_socket,
        }
    }
}

impl Command for WatchNonBlockCommand {
    fn execute(&self) -> Result<(), FzzError> {
        stdout::verbose("Verbose mode enabled.", self.verbose);

        let control_state = Arc::new(Mutex::new(ControlState::default()));
        let coordinator = Arc::new(AwaitCoordinator::new());
        let outputs = Arc::new(OutputRegistry::new());
        let instance = Arc::new(ControlInstance::new());
        let broker = Arc::new(SnapshotBroker::new(
            instance.as_ref().clone(),
            Arc::clone(&control_state),
            Arc::clone(&coordinator),
        ));
        // Duration recorder (TASK-0054): control-run targets record terminal
        // wall durations against their execution signature; fs/init/emit runs
        // carry no signature and are ignored by the recorder.
        let recorder = Arc::new(DurationRecorder::new(DurationStore::new(state_file_path(
            &std::fs::canonicalize(self.watches.root())
                .unwrap_or_else(|_| self.watches.root().to_path_buf()),
            STATE_SCHEMA_VERSION,
        ))));
        let worker_state = Arc::clone(&control_state);
        let coordinator_state = Arc::clone(&coordinator);
        let worker_outputs = Arc::clone(&outputs);
        let broker_state = Arc::clone(&broker);
        let recorder_state = Arc::clone(&recorder);
        let worker = Arc::new(workers::Worker::with_root_and_concurrency_and_outputs(
            self.verbose,
            self.fail_fast,
            self.watches.root().to_path_buf(),
            self.watches.concurrency(),
            move |event| {
                recorder_state.observe(&event);
                coordinator_state.observe(&event);
                worker_state.lock().unwrap().apply(event);
                broker_state.publish();
            },
            Some(worker_outputs),
        ));

        let strategy = NonBlockStrategy::new_arc_with_subscription(
            worker,
            self.watches.clone(),
            self.control_socket.clone(),
            control_state,
            Some(coordinator),
            Some(outputs),
            instance,
            Some(broker),
        );
        watch_loop(&self.watches, self.run_on_init, &*strategy, self.verbose)
    }
}
