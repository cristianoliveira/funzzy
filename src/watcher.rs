extern crate notify;
extern crate notify_debouncer_mini;
use notify_debouncer_mini::notify::ErrorKind;

use self::notify_debouncer_mini::{new_debouncer, notify::RecursiveMode};

use crate::stdout;
use std::path::Path;
use std::sync::mpsc::channel;
use std::time::Duration;

pub fn events(
    watch_path_list: Vec<String>,
    handler: impl Fn(&str),
    verbose: bool,
) -> Result<(), String> {
    let (tx, rx) = channel();
    let mut debouncer =
        new_debouncer(Duration::from_millis(1000), None, tx).expect("Unable to create watcher");
    let watcher = debouncer.watcher();

    for path in watch_path_list {
        if let Err(err) = watcher.watch(Path::new(&path), RecursiveMode::Recursive) {
            let warning = &vec![
                format!("unknown file/directory: '{}'", path),
                format!("Different behaviour depending on the OS."),
                format!("The watcher may not be triggered for this rule."),
            ]
            .join("\n");
            match err.kind {
                ErrorKind::PathNotFound => {
                    stdout::warn(warning);
                }
                ErrorKind::Io(err) => {
                    if err.kind() == std::io::ErrorKind::NotFound {
                        stdout::warn(warning);
                    } else {
                        return Err(format!("failed to watch path: {}\nCause: {:?}", path, err));
                    }
                }
                _ => {
                    return Err(format!("failed to watch path: {}\nCause: {:?}", path, err));
                }
            }
        }
    }

    loop {
        match rx.recv() {
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
                stdout::error(&format!("failed to receive event: {:?}", err));
            }
        }

        std::thread::sleep(std::time::Duration::from_millis(200));
    }
}
