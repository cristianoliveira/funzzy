extern crate notify;

use crate::cli::Command;
use crate::stdout;
use crate::watcher;
use crate::watches::Watches;
use crate::workers;

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

        let worker = workers::Worker::new(self.verbose);

        if let Some(rules) = self.watches.run_on_init() {
            stdout::info("Running on init commands.");
            if let Err(err) = worker.schedule(rules, "") {
                stdout::error(&format!("failed to initiate next run: {:?}", err));
            }
        }

        watcher::events(
            |file_changed| {
                if let Some(rules) = self.watches.watch(file_changed) {
                    if let Err(err) = cmd::execute(&"clear".to_owned()) {
                        stdout::error(&format!("failed to clear screen: {:?}", err));
                    }

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
        );
        stdout::info("Watching...");

        Ok(())
    }
}
