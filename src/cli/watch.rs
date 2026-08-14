use crate::cli::Command;
use crate::errors::FzzError;
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
    events: Option<std::sync::Arc<crate::event_stream::EventStream>>,
}

impl WatchCommand {
    pub fn new(watches: Watches, verbose: bool, fail_fast: bool, run_on_init: bool) -> Self {
        Self::with_events(watches, verbose, fail_fast, run_on_init, None)
    }

    /// Creates the watch command with an optional NDJSON run-event stream
    /// (TASK-0039).
    pub fn with_events(
        watches: Watches,
        verbose: bool,
        fail_fast: bool,
        run_on_init: bool,
        events: Option<std::sync::Arc<crate::event_stream::EventStream>>,
    ) -> Self {
        WatchCommand {
            watches,
            verbose,
            fail_fast,
            run_on_init,
            events,
        }
    }
}

impl Command for WatchCommand {
    fn execute(&self) -> Result<(), FzzError> {
        let strategy = BlockingStrategy::with_events(
            self.watches.root().to_path_buf(),
            self.verbose,
            self.fail_fast,
            self.watches.concurrency(),
            self.events.clone(),
        );
        watch_loop(
            &self.watches,
            self.run_on_init,
            &strategy,
            self.watches.debounce(),
            self.verbose,
        )
    }
}
