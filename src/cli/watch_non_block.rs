use crate::awaiting::AwaitCoordinator;
use crate::cli::Command;
use crate::control::ControlState;
use crate::errors::FzzError;
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
        let worker_state = Arc::clone(&control_state);
        let coordinator_state = Arc::clone(&coordinator);
        let worker = Arc::new(workers::Worker::with_root_and_concurrency(
            self.verbose,
            self.fail_fast,
            self.watches.root().to_path_buf(),
            self.watches.concurrency(),
            move |event| {
                coordinator_state.observe(&event);
                worker_state.lock().unwrap().apply(event);
            },
        ));

        let strategy = NonBlockStrategy::new_arc(
            worker,
            self.watches.clone(),
            self.control_socket.clone(),
            control_state,
            Some(coordinator),
        );
        watch_loop(&self.watches, self.run_on_init, &*strategy, self.verbose)
    }
}
