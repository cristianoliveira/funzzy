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
    fail_fast: bool,
}

impl WatchCommand {
    pub fn new(watches: Watches, verbose: bool, fail_fast: bool) -> Self {
        stdout::verbose(&format!("watches {:?}", watches), verbose);

        WatchCommand {
            watches,
            verbose,
            fail_fast,
        }
    }
}

impl Command for WatchCommand {
    fn execute(&self) -> Result<(), String> {
        stdout::verbose("Verbose mode enabled.", self.verbose);

        if let Some(rules) = self.watches.run_on_init() {
            stdout::info("Running on init commands.");

            let tasks = rules::template(rules::commands(rules), "");
            let mut results: Vec<Result<(), String>> = vec![];
            for task in tasks {
                let result = cmd::execute(&task);

                if self.fail_fast && result.is_err() {
                    results.push(result);
                    break;
                }

                results.push(result);
            }

            stdout::present_results(results);
        } else {
            stdout::info("Watching...");
        }

        watcher::events(
            |file_changed| {
                if let Some(rules) = self.watches.watch(file_changed) {
                    stdout::clear_screen();

                    stdout::verbose(
                        &format!("Triggered by change in: {}", file_changed),
                        self.verbose,
                    );

                    stdout::verbose(&format!("Rules: {:?}", rules), self.verbose);

                    let tasks = rules::template(rules::commands(rules), file_changed);
                    let mut results: Vec<Result<(), String>> = vec![];
                    for task in tasks {
                        let result = cmd::execute(&task);

                        if self.fail_fast && result.is_err() {
                            results.push(result);
                            break;
                        }

                        results.push(result);
                    }

                    stdout::present_results(results);
                }
            },
            self.verbose,
        );
        Ok(())
    }
}
