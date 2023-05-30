extern crate notify;

use std::process::Command as ShellCommand;
use std::sync::mpsc::channel;

use self::notify::{DebouncedEvent, RecommendedWatcher, RecursiveMode, Watcher};
use std::time::Duration;

use cli::Command;
use cmd;
use stdout;
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
            cmd::execute(String::from(command))?
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

        let (tx, rx) = channel();
        let mut watcher: RecommendedWatcher =
            Watcher::new(tx, Duration::from_secs(2)).expect("Unable to create watcher");

        if let Err(err) = watcher.watch(".", RecursiveMode::Recursive) {
            return Err(format!("Unable to watch current directory {:?}", err));
        }

        if let Some(rules) = self.watches.run_on_init() {
            stdout::info(&format!("Running on init commands."));

            if let Err(err) = self.run_rules(rules) {
                return Err(err);
            }

            stdout::info(&"All tasks finished");
        }

        stdout::info(&format!("Watching..."));
        while let Ok(event) = rx.recv() {
            if let DebouncedEvent::Create(path) = event {
                let path_str = path.into_os_string().into_string().unwrap();
                if let Some(rules) = self.watches.watch(&*path_str) {
                    if self.verbose {
                        stdout::verbose(&format!("Triggered by change in: {}", path_str));
                    };

                    self.run_rules(rules)?
                }
            }
        }
        Ok(())
    }
}

fn clear_shell() {
    let _ = ShellCommand::new("clear").status();
}
