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

/// One live root-set swap requested by a config reload (TASK-0090): the
/// complete new root list after commit. The backend diffs it against the
/// currently registered roots and applies `unwatch`/`watch` live, then
/// acknowledges, so there is no event-loss gap between old and new
/// subscriptions (contract §4 commit boundary is synchronous).
#[derive(Clone, Debug)]
pub struct RootSwap {
    pub roots: Vec<String>,
    /// Acknowledgement the backend sends after applying the swap.
    pub ack: Option<std::sync::mpsc::Sender<Result<(), String>>>,
}

/// Receiver side of the root-swap channel fed by the config-reload
/// transaction (TASK-0090).
pub type RootSwapReceiver = std::sync::mpsc::Receiver<RootSwap>;

/// Publishes the complete new root set to the running backend. A no-op when
/// the backend was started without reload support (legacy callers).
#[derive(Clone, Debug)]
pub struct RootSwapPublisher {
    sender: Option<std::sync::mpsc::Sender<RootSwap>>,
}

impl RootSwapPublisher {
    /// A live publisher connected to the running backend.
    pub fn new(sender: std::sync::mpsc::Sender<RootSwap>) -> Self {
        Self {
            sender: Some(sender),
        }
    }

    /// A disabled publisher for backends started without reload support.
    pub fn disabled() -> Self {
        Self { sender: None }
    }

    /// Swaps the live root set synchronously: waits for the backend to apply
    /// the new roots before returning. Returns an error when the backend is
    /// not connected (legacy/disabled), the channel is gone, or the backend
    /// does not acknowledge within the bound.
    pub fn swap(&self, roots: Vec<String>) -> Result<(), String> {
        match &self.sender {
            Some(sender) => {
                let (ack_tx, ack_rx) = std::sync::mpsc::channel();
                sender
                    .send(RootSwap {
                        roots,
                        ack: Some(ack_tx),
                    })
                    .map_err(|_| "root-swap channel closed".to_owned())?;
                ack_rx
                    .recv_timeout(Duration::from_secs(5))
                    .map_err(|_| "root-swap acknowledgement timed out".to_owned())?
            }
            None => Err("backend started without reload support".to_owned()),
        }
    }
}

/// Runs the configured backend with an optional live root-swap channel
/// (TASK-0090). `swap_rx` is consumed by the backend loop: each swap diff is
/// applied to the live watcher without stopping it.
pub fn events(
    watch_path_list: Vec<String>,
    on_ready: impl FnOnce(),
    handler: impl Fn(u64, &[FileEvent]),
    debounce: Duration,
    backend: WatchBackend,
    verbose: bool,
    swap_rx: Option<RootSwapReceiver>,
    shutdown: Option<std::sync::Arc<std::sync::atomic::AtomicBool>>,
) -> Result<(), String> {
    match backend {
        WatchBackend::Native => run_native(
            watch_path_list,
            on_ready,
            handler,
            debounce,
            verbose,
            swap_rx,
            shutdown,
        ),
        WatchBackend::Poll { interval } => run_poll(
            watch_path_list,
            on_ready,
            handler,
            interval,
            swap_rx,
            shutdown,
        ),
        WatchBackend::Auto => {
            // Try native first; on failure warn once and fall back to
            // deterministic polling (TASK-0037). The probe registers the
            // roots without consuming on_ready, so exactly one backend runs.
            match native_available(&watch_path_list) {
                Ok(()) => run_native(
                    watch_path_list,
                    on_ready,
                    handler,
                    debounce,
                    verbose,
                    swap_rx,
                    shutdown,
                ),
                Err(native_err) => {
                    stdout::warn(&format!(
                        "native filesystem backend unavailable ({}); falling back to polling",
                        native_err
                    ));
                    run_poll(
                        watch_path_list,
                        on_ready,
                        handler,
                        Duration::from_millis(500),
                        swap_rx,
                        shutdown,
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
    let mut debouncer = new_debouncer(Duration::from_millis(1000), tx)
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
/// With `swap_rx`, each live root swap is diffed and applied (unwatch/watch)
/// without stopping the backend (TASK-0090).
fn run_native(
    watch_path_list: Vec<String>,
    on_ready: impl FnOnce(),
    handler: impl Fn(u64, &[FileEvent]),
    debounce: Duration,
    verbose: bool,
    swap_rx: Option<RootSwapReceiver>,
    shutdown: Option<std::sync::Arc<std::sync::atomic::AtomicBool>>,
) -> Result<(), String> {
    let (tx, rx) = channel();
    let mut debouncer = new_debouncer(debounce, tx)
        .map_err(|err| format!("unable to create native watcher: {:?}", err))?;
    let watcher = debouncer.watcher();
    let batch_sequence = AtomicSequence::new();

    for path in &watch_path_list {
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

    // A shutdown requested during startup never crosses the readiness gate,
    // so its close hook is ineligible (RUN-HOOKS-CONTRACT §4).
    if shutdown
        .as_ref()
        .is_some_and(|flag| flag.load(std::sync::atomic::Ordering::SeqCst))
    {
        return Ok(());
    }
    // Run initialization only after every path has been registered. Otherwise a
    // fast init command can finish before the watcher is ready, allowing callers
    // to change a file in the gap and lose the first event.
    on_ready();

    let mut current_roots = watch_path_list.clone();
    let mut swap_rx = swap_rx;

    loop {
        if shutdown
            .as_ref()
            .is_some_and(|flag| flag.load(std::sync::atomic::Ordering::SeqCst))
        {
            return Ok(());
        }
        // Apply any pending live root swaps before draining events, so a
        // batch is never routed against a stale root set (contract §4
        // commit boundary).
        if let Some(rx) = swap_rx.as_mut() {
            while let Ok(swap) = rx.try_recv() {
                apply_root_swap(&mut *watcher, &mut current_roots, swap);
            }
        }
        // `recv_timeout` keeps the loop wakeable so pending root swaps are
        // applied even when no filesystem event arrives (TASK-0090). The
        // debounce window is the upper bound on swap latency.
        match rx.recv_timeout(Duration::from_millis(200)) {
            Ok(debounced_evts) => {
                let (events, malformed) = normalize_batch(debounced_evts);
                let events = reconcile_new_directories(events);
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

            Err(err) if matches!(err, std::sync::mpsc::RecvTimeoutError::Timeout) => {
                // Wake-up tick for pending root swaps; not an error.
                continue;
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
    interval: Duration,
    swap_rx: Option<RootSwapReceiver>,
    shutdown: Option<std::sync::Arc<std::sync::atomic::AtomicBool>>,
) -> Result<(), String> {
    let batch_sequence = AtomicSequence::new();
    let mut scanner = PollScanner::new(watch_path_list);
    let mut swap_rx = swap_rx;
    if shutdown
        .as_ref()
        .is_some_and(|flag| flag.load(std::sync::atomic::Ordering::SeqCst))
    {
        return Ok(());
    }
    on_ready();
    loop {
        if shutdown
            .as_ref()
            .is_some_and(|flag| flag.load(std::sync::atomic::Ordering::SeqCst))
        {
            return Ok(());
        }
        // A root swap rebuilds the scanner under the new root set; the next
        // scan seeds the new baseline, so a swap never reports old content
        // as changes (contract §7 parity).
        if let Some(rx) = swap_rx.as_mut() {
            while let Ok(swap) = rx.try_recv() {
                scanner = PollScanner::new(swap.roots.clone());
                if let Some(ack) = swap.ack {
                    let _ = ack.send(Ok(()));
                }
            }
        }
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

/// Applies one live root swap to the native watcher: retires removed roots
/// and registers added roots, updating the tracked set. Called before
/// draining the next debounce batch so routing always sees the committed
/// root set (contract §4 commit boundary).
fn apply_root_swap(
    watcher: &mut dyn notify_debouncer_mini::notify::Watcher,
    current_roots: &mut Vec<String>,
    swap: RootSwap,
) {
    use notify_debouncer_mini::notify::RecursiveMode;
    for root in current_roots.iter() {
        if !swap.roots.contains(root) {
            let _ = watcher.unwatch(Path::new(root));
        }
    }
    for root in &swap.roots {
        if !current_roots.contains(root) {
            let _ = watcher.watch(Path::new(root), RecursiveMode::Recursive);
        }
    }
    *current_roots = swap.roots.clone();
    // Acknowledge so the reload transaction returns only after the backend
    // applies the new roots (contract §4: no event-loss gap at commit).
    if let Some(ack) = swap.ack {
        let _ = ack.send(Ok(()));
    }
}

/// One debounce window is one normalized event batch (contract §1): the
/// changed-path set is deduplicated and deterministically ordered, and each
/// event carries its stable kind. A malformed window yields an empty batch so
/// the watch loop never schedules from an error.
fn normalize_batch(
    debounced: Result<
        Vec<notify_debouncer_mini::DebouncedEvent>,
        notify_debouncer_mini::notify::Error,
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

/// Closes the native-backend registration race (WATCH-DISCOVERY-CONTRACT
/// §4): inotify adds the watch for a newly created directory only when its
/// create event is processed, so files written inside in the same instant
/// are never observed. After each debounced window, every non-continuous
/// event whose path is an existing directory is walked and its descendants
/// are synthesized into the same batch (sorted, deduped), making tree
/// creation observable exactly like the poll backend (§7 equivalence).
/// Continuous (modify) events never rescan, so a touched long-lived
/// directory does not flood the batch with its whole subtree.
fn reconcile_new_directories(mut events: Vec<FileEvent>) -> Vec<FileEvent> {
    let mut known: std::collections::HashSet<String> =
        events.iter().map(|event| event.path.clone()).collect();
    let mut synthesized: Vec<FileEvent> = Vec::new();
    for event in &events {
        if event.continuous {
            continue;
        }
        let path = Path::new(&event.path);
        let is_dir = std::fs::metadata(path)
            .map(|meta| meta.is_dir())
            .unwrap_or(false);
        if !is_dir {
            continue;
        }
        let mut descendants: Vec<PathBuf> = Vec::new();
        walk_descendants(path, &mut descendants);
        for descendant in descendants {
            let Some(descendant_path) = descendant.to_str() else {
                continue;
            };
            if known.insert(descendant_path.to_owned()) {
                synthesized.push(FileEvent {
                    path: descendant_path.to_owned(),
                    continuous: false,
                });
            }
        }
    }
    events.append(&mut synthesized);
    events.sort_by(|a, b| a.path.cmp(&b.path));
    events.dedup_by(|a, b| a.path == b.path);
    events
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
        let (normalized, malformed) = normalize_batch(Err(
            notify_debouncer_mini::notify::Error::generic("test error"),
        ));
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

/// Recursively collects descendants of `root` into `paths`: every child
/// (file or directory), then the children of each non-`.git`, non-symlinked
/// directory. Symlinked directories are recorded as paths but never walked,
/// so cycles cannot recurse (contract §6 symlink policy).
fn walk_descendants(root: &Path, paths: &mut Vec<PathBuf>) {
    let mut stack: Vec<PathBuf> = vec![root.to_path_buf()];
    while let Some(dir) = stack.pop() {
        let Ok(entries) = std::fs::read_dir(&dir) else {
            continue;
        };
        for entry in entries.flatten() {
            let path = entry.path();
            let is_dir = entry
                .file_type()
                .map(|file_type| file_type.is_dir())
                .unwrap_or(false);
            let is_symlink = entry
                .file_type()
                .map(|file_type| file_type.is_symlink())
                .unwrap_or(false);
            let is_git = path.file_name().map(|name| name == ".git").unwrap_or(false);
            paths.push(path.clone());
            if is_dir && !is_symlink && !is_git {
                stack.push(path);
            }
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

    /// Collects every path under the watched roots recursively, in
    /// deterministic order (TASK-0086, contract §7). Does not traverse
    /// `.git`, symlinked directories (cycle safety), or paths outside the
    /// bounded roots; the baseline is seeded on the first scan and never
    /// reported as changes.
    fn collect_paths(&self) -> Vec<PathBuf> {
        let mut paths = vec![];
        for root in &self.watched {
            paths.push(root.clone());
            walk_descendants(root, &mut paths);
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
    fn nested_creation_and_modification_are_detected_recursively() {
        let dir = scratch("nested");
        std::fs::create_dir_all(dir.join("a/b/c")).unwrap();
        std::fs::write(dir.join("a/b/c/deep.txt"), "x").unwrap();
        let mut scanner = PollScanner::new(vec![dir.display().to_string()]);
        scanner.scan(); // baseline seeds the whole tree

        // Create a file two levels below the root: recursive discovery.
        std::fs::write(dir.join("a/b/new.txt"), "y").unwrap();
        let changed = scanner.scan();
        assert!(
            changed.iter().any(|e| e.path.ends_with("a/b/new.txt")),
            "nested create must be detected: {changed:?}"
        );

        // Modify a deeply nested existing file.
        std::thread::sleep(Duration::from_millis(20));
        std::fs::write(dir.join("a/b/c/deep.txt"), "zzz").unwrap();
        let changed = scanner.scan();
        assert!(
            changed.iter().any(|e| e.path.ends_with("a/b/c/deep.txt")),
            "nested modify must be detected: {changed:?}"
        );

        // Remove a nested file.
        std::fs::remove_file(dir.join("a/b/c/deep.txt")).unwrap();
        let changed = scanner.scan();
        assert!(
            changed.iter().any(|e| e.path.ends_with("a/b/c/deep.txt")),
            "nested remove must be detected: {changed:?}"
        );
        std::fs::remove_dir_all(&dir).unwrap();
    }

    #[test]
    fn git_directories_are_not_traversed() {
        let dir = scratch("git-skip");
        std::fs::create_dir_all(dir.join(".git/objects")).unwrap();
        std::fs::write(dir.join(".git/objects/deep.txt"), "x").unwrap();
        std::fs::write(dir.join(".git/config"), "x").unwrap();
        let mut scanner = PollScanner::new(vec![dir.display().to_string()]);
        scanner.scan(); // baseline

        // A change inside .git must never surface (contract §6: no .git
        // traversal).
        std::thread::sleep(Duration::from_millis(20));
        std::fs::write(dir.join(".git/objects/deep.txt"), "zzz").unwrap();
        let changed = scanner.scan();
        assert!(
            !changed.iter().any(|e| e.path.contains(".git")),
            ".git changes must not be reported: {changed:?}"
        );
        std::fs::remove_dir_all(&dir).unwrap();
    }

    #[test]
    fn symlinked_directories_are_recorded_but_not_walked() {
        let dir = scratch("symlink-skip");
        let target = scratch("symlink-target");
        std::fs::create_dir_all(target.join("inner")).unwrap();
        std::fs::write(target.join("inner/real.txt"), "x").unwrap();
        #[cfg(unix)]
        std::os::unix::fs::symlink(&target, dir.join("link")).unwrap();

        let mut scanner = PollScanner::new(vec![dir.display().to_string()]);
        scanner.scan(); // baseline

        // A change inside the symlink target must not surface through the
        // link path (no traversal), and the link itself is not a cycle.
        std::thread::sleep(Duration::from_millis(20));
        std::fs::write(target.join("inner/real.txt"), "zzz").unwrap();
        let changed = scanner.scan();
        assert!(
            !changed.iter().any(|e| e.path.contains("symlink-skip")),
            "symlink traversal must not report target changes: {changed:?}"
        );
        std::fs::remove_dir_all(&dir).unwrap();
        std::fs::remove_dir_all(&target).unwrap();
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

#[cfg(test)]
mod reconcile_tests {
    use super::*;

    fn scratch(name: &str) -> std::path::PathBuf {
        let dir =
            std::env::temp_dir().join(format!("funzzy-reconcile-{}-{}", std::process::id(), name));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        dir
    }

    #[test]
    fn files_created_inside_new_directory_are_synthesized_into_the_batch() {
        // Contract §4: a directory tree + file created in one operation
        // routes on the canonical final file path. inotify registers the
        // watch for a new directory only when its create event is processed,
        // so a file written in the same instant is never observed; the
        // directory event itself is all the batch sees.
        let dir = scratch("synthesize");
        std::fs::create_dir_all(dir.join("src/new")).unwrap();
        std::fs::write(dir.join("src/new/lib.rs"), "x").unwrap();

        let events = vec![FileEvent {
            path: dir.join("src").display().to_string(),
            continuous: false,
        }];
        let reconciled = reconcile_new_directories(events);
        assert!(
            reconciled
                .iter()
                .any(|e| e.path.ends_with("src/new/lib.rs")),
            "file created before watch registration must be synthesized: {reconciled:?}"
        );
        std::fs::remove_dir_all(&dir).unwrap();
    }

    #[test]
    fn reconcile_dedupes_paths_already_observed_in_the_batch() {
        let dir = scratch("dedupe");
        std::fs::create_dir_all(dir.join("tree")).unwrap();
        std::fs::write(dir.join("tree/a.rs"), "x").unwrap();
        std::fs::write(dir.join("tree/b.rs"), "x").unwrap();

        let events = vec![
            FileEvent {
                path: dir.join("tree").display().to_string(),
                continuous: false,
            },
            FileEvent {
                path: dir.join("tree/a.rs").display().to_string(),
                continuous: false,
            },
        ];
        let reconciled = reconcile_new_directories(events);
        let a_count = reconciled
            .iter()
            .filter(|e| e.path.ends_with("tree/a.rs"))
            .count();
        assert_eq!(
            a_count, 1,
            "already observed paths stay unique: {reconciled:?}"
        );
        assert!(
            reconciled.iter().any(|e| e.path.ends_with("tree/b.rs")),
            "sibling created in the same instant is synthesized: {reconciled:?}"
        );
        std::fs::remove_dir_all(&dir).unwrap();
    }

    #[test]
    fn reconcile_skips_continuous_dir_events_and_missing_paths() {
        // A continuous (modify) event on a long-lived directory must not
        // rescan its whole subtree, and removed paths must pass through
        // untouched.
        let dir = scratch("skip-continuous");
        std::fs::create_dir_all(dir.join("old")).unwrap();
        std::fs::write(dir.join("old/pre-existing.rs"), "x").unwrap();

        let events = vec![
            FileEvent {
                path: dir.join("old").display().to_string(),
                continuous: true,
            },
            FileEvent {
                path: dir.join("gone").display().to_string(),
                continuous: false,
            },
        ];
        let reconciled = reconcile_new_directories(events.clone());
        let mut expected = events;
        expected.sort_by(|a, b| a.path.cmp(&b.path));
        assert_eq!(
            reconciled, expected,
            "continuous and nonexistent paths must not synthesize anything"
        );
        std::fs::remove_dir_all(&dir).unwrap();
    }
}

#[cfg(test)]
mod swap_tests {
    use super::*;

    #[test]
    fn publisher_swap_waits_for_backend_ack() {
        let (tx, rx) = std::sync::mpsc::channel();
        let publisher = RootSwapPublisher::new(tx);

        // Backend side: receive the swap, apply, acknowledge.
        let backend = std::thread::spawn(move || {
            let swap = rx.recv_timeout(Duration::from_secs(5)).expect("swap");
            assert_eq!(
                swap.roots,
                vec!["/repo/src".to_owned(), "/repo/docs".to_owned()]
            );
            let ack = swap.ack.expect("ack channel");
            ack.send(Ok(())).unwrap();
        });

        // Publisher side: swap blocks until the ack arrives.
        publisher
            .swap(vec!["/repo/src".to_owned(), "/repo/docs".to_owned()])
            .expect("synchronous swap");
        backend.join().expect("backend");
    }

    #[test]
    fn publisher_swap_errors_when_backend_never_acks() {
        let (tx, _rx) = std::sync::mpsc::channel();
        // Drop the receiver: the sender send fails immediately.
        drop(_rx);
        let publisher = RootSwapPublisher::new(tx);
        let err = publisher
            .swap(vec!["/repo/src".to_owned()])
            .expect_err("closed channel must error");
        assert!(err.contains("closed"), "{err}");
    }

    #[test]
    fn disabled_publisher_errors_without_backend() {
        let publisher = RootSwapPublisher::disabled();
        let err = publisher
            .swap(vec!["/repo/src".to_owned()])
            .expect_err("disabled publisher must error");
        assert!(err.contains("without reload support"), "{err}");
    }
}
