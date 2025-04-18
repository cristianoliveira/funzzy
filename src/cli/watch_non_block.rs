extern crate notify;

use crate::cli::Command;
use crate::errors::FzzError;
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
    fail_fast: bool,
    run_on_init: bool,
}

impl WatchNonBlockCommand {
    pub fn new(watches: Watches, verbose: bool, fail_fast: bool, run_on_init: bool) -> Self {
        stdout::verbose(&format!("watches {:?}", watches), verbose);

        WatchNonBlockCommand {
            watches,
            verbose,
            fail_fast,
            run_on_init,
        }
    }
}

impl Command for WatchNonBlockCommand {
    #![allow(unused_assignments)]
    fn execute(&self) -> Result<(), FzzError> {
        stdout::verbose("Verbose mode enabled.", self.verbose);

        let worker = workers::Worker::new(self.verbose, self.fail_fast, |event| {
            match event {
                workers::WorkerEvent::InitExecution => {}
                workers::WorkerEvent::Tick => {}
                workers::WorkerEvent::FinishedExecution(_) => {}
            };
        });

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
