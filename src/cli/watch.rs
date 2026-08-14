use crate::cli::Command;
use crate::errors::FzzError;
use crate::stdout;
use crate::watch_loop::{watch_loop, BlockingStrategy};
use crate::watches::Watches;

pub const DEFAULT_FILENAME: &str = ".watch.yaml";

/// # `WatchCommand`
///
/// Starts watcher to listen the change events configured
/// in watch.yaml
///
pub struct WatchCommand {
    watches: Watches,
    verbose: bool,
    fail_fast: bool,
    run_on_init: bool,
}

impl WatchCommand {
    pub fn new(watches: Watches, verbose: bool, fail_fast: bool, run_on_init: bool) -> Self {
        stdout::verbose(&watches.diagnostic_summary(), verbose);

        WatchCommand {
            watches,
            verbose,
            fail_fast,
            run_on_init,
        }
    }
}

impl Command for WatchCommand {
    fn execute(&self) -> Result<(), FzzError> {
        stdout::verbose("Verbose mode enabled.", self.verbose);

        let strategy = BlockingStrategy::new(
            self.watches.root().to_path_buf(),
            self.verbose,
            self.fail_fast,
            self.watches.jobs(),
        );
        watch_loop(&self.watches, self.run_on_init, &strategy, self.verbose)
    }
}
