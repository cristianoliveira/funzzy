extern crate notify;

use std::process::Command as ShellCommand;
use std::sync::mpsc::channel;

use self::notify::{DebouncedEvent, RecommendedWatcher, RecursiveMode, Watcher};
use std::time::Duration;

use cli::Command;
use cmd;
use rules;
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
        stdout::verbose(&format!("watches {:?}", watches), verbose);

        WatchCommand { watches, verbose }
    }
}

impl Command for WatchCommand {
    fn execute(&self) -> Result<(), String> {
        stdout::verbose(&format!("Verbose mode enabled."), self.verbose);

        let (tx, rx) = channel();
        let mut watcher: RecommendedWatcher =
            Watcher::new(tx, Duration::from_secs(2)).expect("Unable to create watcher");

        if let Err(err) = watcher.watch(".", RecursiveMode::Recursive) {
            return Err(format!("Unable to watch current directory {:?}", err));
        }

        if let Some(rules) = self.watches.run_on_init() {
            stdout::info(&format!("Running on init commands."));

            let results = rules::as_list(rules)
                .iter()
                .map(|task| {
                    stdout::info(&format!("---- running: {} ----", task));
                    cmd::execute(task)
                })
                .collect::<Vec<Result<(), String>>>();

            stdout::present_results(results);
        }

        stdout::info(&format!("Watching..."));
        while let Ok(event) = rx.recv() {
            if let DebouncedEvent::Create(path) = event {
                let path_str = path.into_os_string().into_string().unwrap();
                if let Some(rules) = self.watches.watch(&*path_str) {
                    stdout::verbose(
                        &format!("Triggered by change in: {}", path_str),
                        self.verbose,
                    );

                    let results = rules::as_list(rules)
                        .iter()
                        .map(|task| {
                            stdout::info(&format!("---- running: {} ----", task));
                            cmd::execute(task)
                        })
                        .collect::<Vec<Result<(), String>>>();

                    stdout::present_results(results);
                }
            }
        }
        Ok(())
    }
}
