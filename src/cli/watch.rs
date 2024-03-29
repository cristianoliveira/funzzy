use crate::cli::Command;
use crate::cmd;
use crate::rules;
use crate::stdout;
use crate::watcher;
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
}

impl WatchCommand {
    pub fn new(watches: Watches, verbose: bool) -> Self {
        stdout::verbose(&format!("watches {:?}", watches), verbose);

        WatchCommand { watches, verbose }
    }
}

impl Command for WatchCommand {
    fn execute(&self) -> Result<(), String> {
        stdout::verbose("Verbose mode enabled.", self.verbose);

        if let Some(rules) = self.watches.run_on_init() {
            stdout::info("Running on init commands.");

            let results = rules::commands(rules)
                .iter()
                .map(cmd::execute)
                .collect::<Vec<Result<(), String>>>();

            stdout::present_results(results);
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

                    stdout::verbose(&format!("Rules: {:?}", rules), self.verbose);

                    let results = rules::template(rules::commands(rules), file_changed)
                        .iter()
                        .map(cmd::execute)
                        .collect::<Vec<Result<(), String>>>();
                    stdout::present_results(results);
                }
            },
            self.verbose,
        );
        Ok(())
    }
}
