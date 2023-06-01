extern crate notify;

use std::sync::mpsc::channel;
use std::sync::mpsc::TryRecvError;

use self::notify::{DebouncedEvent, RecommendedWatcher, RecursiveMode, Watcher};
use std::time::Duration;

use cli::Command;
use stdout;
use watches::Watches;
use workers;

/// # `WatchNonBlockCommand`
///
/// Starts watcher to listen the change events configured
/// in watch.yaml, upon change it cancell all previous tasks, including the running
/// task and initiate a new set of tasks.
///
pub struct WatchNonBlockCommand {
    watches: Watches,
    verbose: bool,
}

impl WatchNonBlockCommand {
    pub fn new(watches: Watches, verbose: bool) -> Self {
        stdout::verbose(&format!("watches {:?}", watches), verbose);

        WatchNonBlockCommand { watches, verbose }
    }
}

impl Command for WatchNonBlockCommand {
    fn execute(&self) -> Result<(), String> {
        stdout::verbose("Verbose mode enabled.", self.verbose);

        let (tx, rx) = channel();
        let mut watcher: RecommendedWatcher =
            Watcher::new(tx, Duration::from_secs(2)).expect("Unable to create watcher");

        if let Err(err) = watcher.watch(".", RecursiveMode::Recursive) {
            return Err(format!("Unable to watch current directory {:?}", err));
        }

        let worker = workers::Worker::new(self.verbose);

        if let Some(rules) = self.watches.run_on_init() {
            stdout::info("Running on init commands.");
            if let Err(err) = worker.schedule(rules) {
                stdout::error(&format!("failed to initiate next run: {:?}", err));
            }
        }

        stdout::info("Watching...");
        loop {
            match rx.try_recv() {
                Ok(event) => {
                    if let DebouncedEvent::Create(path) = event {
                        let path_str = path.into_os_string().into_string().unwrap();

                        stdout::verbose(&format!("Changed file: {}", path_str), self.verbose);

                        if let Some(rules) = self.watches.watch(&path_str) {
                            stdout::verbose(
                                &format!("Triggered by change in: {}", path_str),
                                self.verbose,
                            );

                            if let Err(err) = worker.cancel_running_tasks() {
                                stdout::error(&format!(
                                    "failed to cancel current running tasks: {:?}",
                                    err
                                ));
                            }

                            if let Err(err) = worker.schedule(rules.clone()) {
                                stdout::error(&format!("failed to initiate next run: {:?}", err));
                            }
                        }
                    }
                }

                Err(err) => {
                    if err != TryRecvError::Empty {
                        stdout::error(&format!("failed to receive event: {:?}", err));
                        break;
                    }
                }
            }
        }

        Ok(())
    }
}
