use cli::Command;
use cmd;
use rules;
use stdout;
use watcher;
use watches::Watches;

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
                .map(|task| {
                    stdout::info(&format!("task {} \n", String::from(task)));
                    cmd::execute(task)
                })
                .collect::<Vec<Result<(), String>>>();

            stdout::present_results(results);
        }

        watcher::events(
            |file_changed| {
                if let Some(rules) = self.watches.watch(&file_changed) {
                    stdout::verbose(
                        &format!("Triggered by change in: {}", file_changed),
                        self.verbose,
                    );

                    let results = rules::template(rules::commands(rules), file_changed)
                        .iter()
                        .map(|task| {
                            stdout::info(&format!(" task {} \n", String::from(task)));
                            cmd::execute(task)
                        })
                        .collect::<Vec<Result<(), String>>>();
                    stdout::present_results(results);
                }
            },
            self.verbose,
        );
        Ok(())
    }
}
