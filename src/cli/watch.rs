extern crate notify;

use std::process::Command as ShellCommand;

use cli::Command;
use cmd;
use stdout;
use watchers::*;
use watches::Watches;

pub const DEFAULT_FILENAME: &'static str = ".watch.yaml";

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
        if verbose {
            stdout::verbose(&format!("watches {:?}", watches));
        }

        WatchCommand { watches, verbose }
    }

    fn run(&self, commands: &Vec<String>) -> Result<(), String> {
        for command in commands {
            if self.verbose {
                stdout::verbose(&format!("struct command: {:?}", command));
            };
            stdout::info(&format!("----- command: {} -------", command));
            if let Err(err) = cmd::execute(String::from(command)) {
                stdout::error(&format!("failed to run command: {:?}", err));
                return Err(err);
            }
        }
        Ok(())
    }

    fn run_rules(&self, rules: Vec<Vec<String>>) -> Result<(), String> {
        clear_shell();
        let results = rules
            .iter()
            .map(|rule_cmds| self.run(&rule_cmds))
            .find(|r| match r {
                Ok(_) => false,
                Err(_) => true,
            });

        match results {
            Some(Err(err)) => Err(err),
            _ => Ok(()),
        }
    }
}

impl Command for WatchCommand {
    fn execute(&self) -> Result<(), String> {
        if self.verbose {
            stdout::verbose(&format!("Verbose mode enabled."));
        };

        if let Some(rules) = self.watches.run_on_init() {
            stdout::info(&format!("Running on init commands."));

            if let Err(err) = self.run_rules(rules) {
                return Err(err);
            }

            stdout::info(&"All tasks finished");
        }

        let rule_watcher = RulesWatcher::new(self.watches, self.verbose);

        stdout::info(&format!("Watching..."));
        loop {
            if let Err(err) = rule_watcher.on_event(Box::new(|rules| {
                if let Some(rules) = rules {
                    if let Err(err) = self.run_rules(rules) {
                        stdout::error(&format!("Unable to run rules {:?}", err));
                    }
                }
            })) {
                stdout::error(&format!("Unable to watch current directory {:?}", err));
                return Err(err);
            }

            break;
        }

        Ok(())
    }
}

fn clear_shell() {
    let _ = ShellCommand::new("clear").status();
}
