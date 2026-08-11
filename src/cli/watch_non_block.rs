extern crate notify;

use crate::cli::Command;
use crate::control::{ControlServer, ControlState};
use crate::errors::FzzError;
use crate::stdout;
use crate::watcher;
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
        stdout::verbose(&format!("watches {:?}", watches), verbose);

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
    #![allow(unused_assignments)]
    fn execute(&self) -> Result<(), FzzError> {
        stdout::verbose("Verbose mode enabled.", self.verbose);

        let control_state = Arc::new(Mutex::new(ControlState::default()));
        let worker_state = Arc::clone(&control_state);
        let worker = Arc::new(workers::Worker::new(
            self.verbose,
            self.fail_fast,
            move |event| {
                worker_state.lock().unwrap().apply(event);
            },
        ));
        let _control_server = if let Some(path) = self.control_socket.as_ref() {
            let runner_worker = Arc::clone(&worker);
            let runner_watches = self.watches.clone();
            Some(
                ControlServer::start_with_runner(path, Arc::clone(&control_state), move |target| {
                    let rules = runner_watches
                        .target(&target)
                        .ok_or_else(|| format!("No target found for '{}'", target))?;
                    runner_worker.cancel_running_tasks()?;
                    runner_worker.schedule(rules, &format!("control:{}", target))
                })
                .map_err(|err| FzzError::GenericError(err.to_string()))?,
            )
        } else {
            None
        };

        if let Some(rules) = self.watches.run_on_init() {
            if self.run_on_init {
                stdout::info("Running on init commands.");
                if let Err(err) = worker.schedule(rules, "") {
                    stdout::error(&format!("failed to initiate next run: {:?}", err));
                }
            } else {
                stdout::info("Watching...");
            }
        } else {
            stdout::info("Watching...");
        }

        let list_of_watched_paths = self.watches.paths_to_watch().unwrap_or_default();
        match watcher::events(
            list_of_watched_paths,
            |file_changed| {
                if let Some(rules) = self.watches.watch(file_changed) {
                    stdout::clear_screen();

                    stdout::verbose(
                        &format!("Triggered by change in: {}", file_changed),
                        self.verbose,
                    );

                    if let Err(err) = worker.cancel_running_tasks() {
                        stdout::error(&format!(
                            "failed to cancel current running tasks: {:?}",
                            err
                        ));
                    }

                    if let Err(err) = worker.schedule(rules, file_changed) {
                        stdout::error(&format!("failed to initiate next run: {:?}", err));
                    }
                }
            },
            self.verbose,
        ) {
            Ok(_) => Ok(()),
            Err(err) => Err(FzzError::GenericError(err)),
        }
    }
}
