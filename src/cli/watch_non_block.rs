extern crate notify;

use crate::cli::Command;
use crate::control::{ControlServer, ControlState, ControlTarget};
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

    /// Starts the control server when a socket is configured.
    ///
    /// The control socket is an auxiliary control surface, so a bind failure
    /// (for example, another live instance already owns the socket) must NOT
    /// bring the watcher down. We log a warning and continue without it.
    fn start_control_server(
        &self,
        worker: &Arc<workers::Worker>,
        control_state: &Arc<Mutex<ControlState>>,
    ) -> Option<ControlServer> {
        let path = self.control_socket.as_ref()?;

        let runner_worker = Arc::clone(worker);
        let runner_watches = self.watches.clone();
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
            Arc::clone(control_state),
            targets,
            move |target| {
                stdout::info(&format!("Control requested target: {}", target));
                let rules = runner_watches
                    .target(&target)
                    .ok_or_else(|| format!("No target found for '{}'", target))?;
                runner_worker.cancel_running_tasks()?;
                runner_worker.schedule(rules, &format!("control:{}", target))
            },
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
        let list_of_watched_paths = self.watches.paths_to_watch().unwrap_or_default();
        let mut _control_server = None;
        match watcher::events(
            list_of_watched_paths,
            || {
                // Publish the control socket and start initial work only after
                // filesystem watches are registered, so readiness is truthful.
                _control_server = self.start_control_server(&worker, &control_state);

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
            },
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

#[cfg(test)]
mod tests {
    use super::*;
    use crate::control::ControlState;
    use crate::watches::Watches;
    use std::sync::{Arc, Mutex};

    fn unique_socket(label: &str) -> std::path::PathBuf {
        std::env::temp_dir().join(format!("funzzy-wnb-{}-{}.sock", std::process::id(), label))
    }

    #[test]
    fn it_continues_without_a_control_socket_when_it_is_already_in_use() {
        let path = unique_socket("conflict");

        // A live instance already owns the socket.
        let holder_state = Arc::new(Mutex::new(ControlState::default()));
        let _holder = ControlServer::start(&path, holder_state).unwrap();

        let cmd =
            WatchNonBlockCommand::new(Watches::new(vec![]), false, false, true, Some(path.clone()));
        let worker = Arc::new(workers::Worker::new(false, false, |_| {}));
        let control_state = Arc::new(Mutex::new(ControlState::default()));

        // The watcher must NOT die just because its control socket is taken.
        let server = cmd.start_control_server(&worker, &control_state);
        assert!(
            server.is_none(),
            "control server startup must be non-fatal when the socket is already in use"
        );
    }

    #[test]
    fn it_starts_the_control_server_when_the_socket_is_free() {
        let path = unique_socket("free");

        let cmd =
            WatchNonBlockCommand::new(Watches::new(vec![]), false, false, true, Some(path.clone()));
        let worker = Arc::new(workers::Worker::new(false, false, |_| {}));
        let control_state = Arc::new(Mutex::new(ControlState::default()));

        let server = cmd.start_control_server(&worker, &control_state);
        assert!(
            server.is_some(),
            "control server should start when the socket is free"
        );
        assert!(path.exists(), "the socket file should be created");
    }
}
