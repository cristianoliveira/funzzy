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
    eprintln!("DEBUG: watch_path_list: {:?}", watch_path_list);

    for path in watch_path_list {
        stdout::verbose(&format!("Watching path: {}", path), verbose);
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
                stdout::verbose(&format_events(&debounced_evts), verbose);
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

pub fn format_events(
    events: &Result<
        Vec<notify_debouncer_mini::DebouncedEvent>,
        Vec<notify_debouncer_mini::notify::Error>,
    >,
) -> String {
    let events_formatted = events
        .iter()
        .flatten()
        .map(|e| format!("--: {}", e.path.display()))
        .collect::<Vec<String>>()
        .join("\n");

    format!("Events formatted: {}\n", events_formatted)
}
