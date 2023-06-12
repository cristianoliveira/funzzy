extern crate notify;
extern crate notify_debouncer_mini;
use self::notify_debouncer_mini::{new_debouncer, notify::RecursiveMode};

use std::path::Path;
use std::sync::mpsc::channel;
use std::sync::mpsc::TryRecvError;
use std::time::Duration;
use stdout;

pub fn events(handler: impl Fn(&str), verbose: bool) {
    let (tx, rx) = channel();
    let mut debouncer =
        new_debouncer(Duration::from_millis(1000), None, tx).expect("Unable to create watcher");
    let watcher = debouncer.watcher();

    if let Err(err) = watcher.watch(Path::new("."), RecursiveMode::Recursive) {
        println!("Unable to watch current directory {:?}", err);
    }

    stdout::info("Watching...");
    loop {
        match rx.try_recv() {
            Ok(debounced_evts) => {
                stdout::verbose(&format!("Events {:?}", debounced_evts), verbose);
                if let Ok(file_change_event) = debounced_evts {
                    file_change_event.iter().for_each(|event| {
                        if let Some(path_string) = event.path.to_str() {
                            handler(path_string);
                        } else {
                            stdout::error(&format!(
                                "failed to convert path {:?} to string",
                                event.path
                            ));
                        }
                    });
                }
            }

            Err(err) => {
                if err != TryRecvError::Empty {
                    stdout::error(&format!("failed to receive event: {:?}", err));
                    break;
                }
            }
        }

        std::thread::sleep(std::time::Duration::from_millis(200));
    }
}
