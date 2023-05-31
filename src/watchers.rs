extern crate notify;

use self::notify::{DebouncedEvent, RecommendedWatcher, RecursiveMode, Watcher};
use std::sync::mpsc::channel;
use std::time::Duration;

use rules::Rules;
use stdout;
use watches::Watches;

pub trait EventWatcher<T> {
    fn on_event(self, handler: Box<dyn Fn(Option<T>) -> ()>) -> Result<(), String>;
}

pub struct RulesWatcher {
    watches: Watches,
    verbose: bool,
}

impl RulesWatcher {
    pub fn new(watches: Watches, verbose: bool) -> Self {
        if verbose {
            stdout::verbose(&format!("watches {:?}", watches));
        }

        RulesWatcher { watches, verbose }
    }
}

impl EventWatcher<Vec<Rules>> for RulesWatcher {
    fn on_event(self, handler: Box<dyn Fn(Option<Vec<Rules>>) -> ()>) -> Result<(), String> {
        let (tx, rx) = channel();
        let mut watcher: RecommendedWatcher =
            Watcher::new(tx, Duration::from_secs(2)).expect("Unable to create watcher");

        if let Err(err) = watcher.watch(".", RecursiveMode::Recursive) {
            return Err(format!("Unable to watch current directory {:?}", err));
        }

        loop {
            match rx.recv() {
                Ok(event) => {
                    if let DebouncedEvent::Create(path) = event {
                        let path_str = path.into_os_string().into_string().unwrap();
                        let rules = self.watches.watching(&*path_str);

                        if self.verbose {
                            stdout::verbose(&format!("Triggered by change in: {}", path_str));
                        };

                        handler(rules);
                    }
                }

                Err(err) => {
                    return Err(format!("Unable to watch current directory {:?}", err));
                }
            }
        }
    }
}
