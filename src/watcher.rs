extern crate notify;
extern crate notify_debouncer_mini;
use notify_debouncer_mini::notify::ErrorKind;

use self::notify_debouncer_mini::{new_debouncer, notify::RecursiveMode};

use crate::diagnostics;
use crate::identity::AtomicSequence;
use crate::stdout;
use std::path::Path;
use std::sync::mpsc::channel;
use std::time::Duration;

/// One debounced filesystem event forwarded to the watch loop: the raw path
/// and whether the debounce window considered it continuous. The event kind
/// vocabulary stays stable (`any` / `continuous`) so diagnostics never depend
/// on notify internals.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct FileEvent {
    pub path: String,
    /// True when the debounce window saw the path as continuously written.
    pub continuous: bool,
}

/// Watches every path, then forwards one normalized event batch per debounce
/// window. The batch identity is assigned from a fresh per-instance sequence,
/// so one window maps to zero or one generation (contract §1) and the same
/// identity correlates the diagnostics of the whole decision chain.
pub fn events(
    watch_path_list: Vec<String>,
    on_ready: impl FnOnce(),
    handler: impl Fn(u64, &[FileEvent]),
    debounce: Duration,
    verbose: bool,
) -> Result<(), String> {
    let (tx, rx) = channel();
    let mut debouncer = new_debouncer(debounce, None, tx).expect("Unable to create watcher");
    let watcher = debouncer.watcher();
    let batch_sequence = AtomicSequence::new();

    for path in watch_path_list {
        if verbose {
            diagnostics::debug(&diagnostics::Record {
                source: Some("config"),
                decision: Some("watch_root"),
                path: Some(path.clone()),
                ..Default::default()
            });
        }
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

    // Run initialization only after every path has been registered. Otherwise a
    // fast init command can finish before the watcher is ready, allowing callers
    // to change a file in the gap and lose the first event.
    on_ready();

    loop {
        match rx.recv() {
            Ok(debounced_evts) => {
                let (events, malformed) = normalize_batch(debounced_evts);
                if events.is_empty() {
                    // Malformed or empty windows never schedule; still surface
                    // the observation when diagnostics are enabled.
                    if verbose && malformed {
                        diagnostics::debug(&diagnostics::Record {
                            source: Some("filesystem"),
                            decision: Some("error"),
                            note: Some("malformed watcher event batch".to_owned()),
                            ..Default::default()
                        });
                    }
                    continue;
                }
                let batch_id = batch_sequence.next();
                if verbose {
                    // One deterministic batch summary (TASK-0031): batch
                    // identity, debounce window, and normalized size, so the
                    // whole collapse is observable, not just individual events.
                    diagnostics::debug(&diagnostics::Record {
                        batch: Some(batch_id),
                        source: Some("filesystem"),
                        decision: Some("batch"),
                        note: Some(format!(
                            "{} normalized path(s) in a {:?} debounce window",
                            events.len(),
                            debounce
                        )),
                        ..Default::default()
                    });
                    for event in &events {
                        diagnostics::debug(&diagnostics::Record {
                            batch: Some(batch_id),
                            source: Some("filesystem"),
                            kind: Some(if event.continuous {
                                "continuous"
                            } else {
                                "any"
                            }),
                            path: Some(event.path.clone()),
                            normalized: Some(event.path.clone()),
                            decision: Some("event"),
                            ..Default::default()
                        });
                    }
                }
                handler(batch_id, &events);
            }

            Err(err) => {
                if verbose {
                    diagnostics::debug(&diagnostics::Record {
                        source: Some("filesystem"),
                        decision: Some("error"),
                        note: Some(format!("failed to receive event: {:?}", err)),
                        ..Default::default()
                    });
                }
                stdout::error(&format!("failed to receive event: {:?}", err));
            }
        }

        std::thread::sleep(std::time::Duration::from_millis(200));
    }
}

/// One debounce window is one normalized event batch (contract §1): the
/// changed-path set is deduplicated and deterministically ordered, and each
/// event carries its stable kind. A malformed window yields an empty batch so
/// the watch loop never schedules from an error.
fn normalize_batch(
    debounced: Result<
        Vec<notify_debouncer_mini::DebouncedEvent>,
        Vec<notify_debouncer_mini::notify::Error>,
    >,
) -> (Vec<FileEvent>, bool) {
    let Ok(file_change_event) = debounced else {
        return (vec![], true);
    };
    let mut events: Vec<FileEvent> = file_change_event
        .iter()
        .filter_map(|event| {
            event.path.to_str().map(|path| FileEvent {
                path: path.to_owned(),
                continuous: matches!(
                    event.kind,
                    notify_debouncer_mini::DebouncedEventKind::AnyContinuous
                ),
            })
        })
        .collect();
    events.sort_by(|a, b| a.path.cmp(&b.path));
    events.dedup_by(|a, b| a.path == b.path);
    (events, false)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn batch_normalization_dedupes_sorts_and_keeps_kind() {
        use notify_debouncer_mini::DebouncedEvent;
        use notify_debouncer_mini::DebouncedEventKind;

        let events = Ok(vec![
            DebouncedEvent {
                path: std::path::PathBuf::from("b.txt"),
                kind: DebouncedEventKind::AnyContinuous,
            },
            DebouncedEvent {
                path: std::path::PathBuf::from("a.txt"),
                kind: DebouncedEventKind::Any,
            },
            DebouncedEvent {
                path: std::path::PathBuf::from("b.txt"),
                kind: DebouncedEventKind::Any,
            },
        ]);
        let (normalized, malformed) = normalize_batch(events);
        assert!(!malformed);
        assert_eq!(
            normalized,
            vec![
                FileEvent {
                    path: "a.txt".to_owned(),
                    continuous: false,
                },
                FileEvent {
                    path: "b.txt".to_owned(),
                    continuous: true,
                },
            ]
        );
    }

    #[test]
    fn malformed_window_yields_empty_batch() {
        let (normalized, malformed) = normalize_batch(Err(vec![]));
        assert!(malformed);
        assert!(normalized.is_empty());
    }

    #[test]
    fn empty_window_yields_empty_batch() {
        let (normalized, malformed) = normalize_batch(Ok(vec![]));
        assert!(!malformed);
        assert!(normalized.is_empty());
    }
}
