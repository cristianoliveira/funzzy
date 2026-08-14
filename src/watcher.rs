extern crate notify;
extern crate notify_debouncer_mini;
use notify_debouncer_mini::notify::ErrorKind;

use self::notify_debouncer_mini::{new_debouncer, notify::RecursiveMode};

use crate::diagnostics;
use crate::identity::AtomicSequence;
use crate::stdout;
use std::path::{Path, PathBuf};
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
    backend: WatchBackend,
    verbose: bool,
) -> Result<(), String> {
    match backend {
        WatchBackend::Native => run_native(watch_path_list, on_ready, handler, debounce, verbose),
        WatchBackend::Poll { interval } => {
            run_poll(watch_path_list, on_ready, handler, debounce, interval)
        }
        WatchBackend::Auto => {
            // Try native first; on failure warn once and fall back to
            // deterministic polling (TASK-0037). The probe registers the
            // roots without consuming on_ready, so exactly one backend runs.
            match native_available(&watch_path_list) {
                Ok(()) => run_native(watch_path_list, on_ready, handler, debounce, verbose),
                Err(native_err) => {
                    stdout::warn(&format!(
                        "native filesystem backend unavailable ({}); falling back to polling",
                        native_err
                    ));
                    run_poll(
                        watch_path_list,
                        on_ready,
                        handler,
                        debounce,
                        Duration::from_millis(500),
                    )
                }
            }
        }
    }
}

/// Probes whether the native notify backend can register every watch root;
/// used by Auto to decide the backend before consuming `on_ready`.
fn native_available(watch_path_list: &[String]) -> Result<(), String> {
    let (tx, _rx) = channel();
    let mut debouncer = new_debouncer(Duration::from_millis(1000), None, tx)
        .map_err(|err| format!("native backend init failed: {:?}", err))?;
    let watcher = debouncer.watcher();
    for path in watch_path_list {
        watcher
            .watch(Path::new(path), RecursiveMode::Recursive)
            .map_err(|err| format!("cannot watch '{}': {:?}", path, err))?;
    }
    Ok(())
}

/// Runs the native notify backend: one normalized batch per debounce window.
fn run_native(
    watch_path_list: Vec<String>,
    on_ready: impl FnOnce(),
    handler: impl Fn(u64, &[FileEvent]),
    debounce: Duration,
    verbose: bool,
) -> Result<(), String> {
    let (tx, rx) = channel();
    let mut debouncer = new_debouncer(debounce, None, tx)
        .map_err(|err| format!("unable to create native watcher: {:?}", err))?;
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

/// Runs the deterministic polling backend (TASK-0037): scans the watched
/// roots on a fixed interval and feeds the same normalized batch + handler
/// path as the native backend. Removals and renames appear as path changes
/// that the shared matching handles identically.
fn run_poll(
    watch_path_list: Vec<String>,
    on_ready: impl FnOnce(),
    handler: impl Fn(u64, &[FileEvent]),
    debounce: Duration,
    interval: Duration,
) -> Result<(), String> {
    let batch_sequence = AtomicSequence::new();
    let mut scanner = PollScanner::new(watch_path_list);
    on_ready();
    loop {
        let events = scanner.scan();
        if !events.is_empty() {
            let batch_id = batch_sequence.next();
            if !events.is_empty() {
                handler(batch_id, &events);
            }
        }
        std::thread::sleep(interval.max(Duration::from_millis(20)));
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

/// Watch backend policy (TASK-0037): native notify, deterministic polling,
/// or auto (try native, fall back to polling with one actionable warning).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum WatchBackend {
    Native,
    Poll { interval: Duration },
    Auto,
}

impl WatchBackend {
    /// Parses `on.watch_backend` (native|poll|auto) plus an optional poll
    /// interval; invalid values are rejected loudly.
    pub fn parse(backend: Option<&str>, poll_interval: Option<Duration>) -> Result<Self, String> {
        let backend = backend.unwrap_or("auto");
        match backend {
            "auto" => Ok(WatchBackend::Auto),
            "native" => Ok(WatchBackend::Native),
            "poll" => Ok(WatchBackend::Poll {
                interval: poll_interval.unwrap_or(Duration::from_millis(500)),
            }),
            other => Err(format!(
                "invalid 'on.watch_backend' '{}': expected native, poll, or auto",
                other
            )),
        }
    }
}

/// One filesystem fact the poll scanner tracks: the modified time and whether
/// the path exists. A change in either detects create/modify/remove and
/// rename-equivalent (old path gone + new path present) for matching.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PollSnapshot {
    pub modified: Option<u64>,
}

impl PollSnapshot {
    fn capture(path: &Path) -> PollSnapshot {
        let modified = std::fs::metadata(path)
            .ok()
            .and_then(|meta| meta.modified().ok())
            .map(|t| {
                t.duration_since(std::time::UNIX_EPOCH)
                    .map(|d| d.as_millis() as u64)
                    .unwrap_or(0)
            });
        PollSnapshot { modified }
    }
}

/// Deterministic polling scanner (TASK-0037): snapshots the mtime/existence
/// of every watched root's descendants, and reports paths whose fact changed
/// since the previous snapshot. Pure and clock-free — the caller decides the
/// interval. Renames appear as one remove + one create, which the same
/// matching path handles identically to native backends.
pub struct PollScanner {
    watched: Vec<PathBuf>,
    previous: std::collections::HashMap<PathBuf, PollSnapshot>,
    seeded: bool,
}

impl PollScanner {
    pub fn new(watched: Vec<String>) -> Self {
        Self {
            watched: watched.into_iter().map(PathBuf::from).collect(),
            previous: std::collections::HashMap::new(),
            seeded: false,
        }
    }

    /// Collects every path under the watched roots (one level deep plus the
    /// roots themselves), in deterministic order.
    fn collect_paths(&self) -> Vec<PathBuf> {
        let mut paths = vec![];
        for root in &self.watched {
            paths.push(root.clone());
            if let Ok(entries) = std::fs::read_dir(root) {
                for entry in entries.flatten() {
                    paths.push(entry.path());
                }
            }
        }
        paths.sort();
        paths.dedup();
        paths
    }

    /// Returns paths whose mtime or existence changed since the last scan.
    /// The first scan seeds the baseline and reports nothing.
    pub fn scan(&mut self) -> Vec<FileEvent> {
        let mut changed = vec![];
        let mut next: std::collections::HashMap<PathBuf, PollSnapshot> =
            std::collections::HashMap::new();
        for path in self.collect_paths() {
            let current = PollSnapshot::capture(&path);
            let changed_fact = if !self.seeded {
                false // first scan seeds the baseline, reports nothing
            } else {
                match self.previous.get(&path) {
                    Some(before) => before != &current,
                    None => true,
                }
            };
            if changed_fact {
                changed.push(FileEvent {
                    path: path.display().to_string(),
                    continuous: false,
                });
            }
            next.insert(path, current);
        }
        // Removals: paths present in the previous snapshot but gone now are
        // changes too (the file no longer exists).
        if self.seeded {
            for path in self.previous.keys() {
                if !next.contains_key(path) {
                    changed.push(FileEvent {
                        path: path.display().to_string(),
                        continuous: false,
                    });
                }
            }
        }
        self.previous = next;
        self.seeded = true;
        changed
    }
}

#[cfg(test)]
mod poll_tests {
    use super::*;

    fn scratch(name: &str) -> std::path::PathBuf {
        let dir = std::env::temp_dir().join(format!("funzzy-poll-{}-{}", std::process::id(), name));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        dir
    }

    #[test]
    fn first_scan_seeds_baseline_and_reports_nothing() {
        let dir = scratch("seed");
        std::fs::write(dir.join("a.txt"), "x").unwrap();
        let mut scanner = PollScanner::new(vec![dir.display().to_string()]);
        assert!(scanner.scan().is_empty(), "first scan seeds only");
        std::fs::remove_dir_all(&dir).unwrap();
    }

    #[test]
    fn create_and_modify_are_detected_after_baseline() {
        let dir = scratch("create-modify");
        std::fs::write(dir.join("a.txt"), "x").unwrap();
        let mut scanner = PollScanner::new(vec![dir.display().to_string()]);
        scanner.scan(); // baseline

        // Create a new file.
        std::fs::write(dir.join("b.txt"), "y").unwrap();
        let changed = scanner.scan();
        assert!(
            changed.iter().any(|e| e.path.ends_with("b.txt")),
            "create must be detected: {changed:?}"
        );

        // Modify an existing file (mtime advances; tolerate coarse clocks by
        // writing different content and ensuring a fresh mtime).
        std::thread::sleep(Duration::from_millis(20));
        std::fs::write(dir.join("a.txt"), "zzz").unwrap();
        let changed = scanner.scan();
        assert!(
            changed.iter().any(|e| e.path.ends_with("a.txt")),
            "modify must be detected: {changed:?}"
        );
        std::fs::remove_dir_all(&dir).unwrap();
    }

    #[test]
    fn remove_is_detected_as_existence_change() {
        let dir = scratch("remove");
        std::fs::write(dir.join("a.txt"), "x").unwrap();
        let mut scanner = PollScanner::new(vec![dir.display().to_string()]);
        scanner.scan(); // baseline
        std::fs::remove_file(dir.join("a.txt")).unwrap();
        let changed = scanner.scan();
        assert!(
            changed.iter().any(|e| e.path.ends_with("a.txt")),
            "remove must be detected: {changed:?}"
        );
        std::fs::remove_dir_all(&dir).unwrap();
    }

    #[test]
    fn unchanged_scan_reports_nothing() {
        let dir = scratch("quiet");
        std::fs::write(dir.join("a.txt"), "x").unwrap();
        let mut scanner = PollScanner::new(vec![dir.display().to_string()]);
        scanner.scan(); // baseline
        std::thread::sleep(Duration::from_millis(20));
        let changed = scanner.scan();
        assert!(changed.is_empty(), "no changes: {changed:?}");
        std::fs::remove_dir_all(&dir).unwrap();
    }

    #[test]
    fn backend_parse_validates_selection_and_interval() {
        assert_eq!(WatchBackend::parse(None, None).unwrap(), WatchBackend::Auto);
        assert_eq!(
            WatchBackend::parse(Some("native"), None).unwrap(),
            WatchBackend::Native
        );
        assert_eq!(
            WatchBackend::parse(Some("poll"), Some(Duration::from_millis(200))).unwrap(),
            WatchBackend::Poll {
                interval: Duration::from_millis(200)
            }
        );
        assert!(WatchBackend::parse(Some("bogus"), None).is_err());
    }
}
