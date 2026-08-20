//! Live configuration reload session.
//!
//! Owns config-file event filtering, candidate validation, and the
//! prepare/commit/retire lifecycle. Application composition wires and joins
//! the session; reload policy does not live in the CLI dispatcher.

use crate::reload_coordinator::ReloadCoordinator;
use crate::shutdown::ShutdownCoordinator;
use crate::watches::Watches;
use crate::{config, logging, stdout, watcher};
use std::path::PathBuf;
use std::sync::{mpsc, Arc};
use std::thread::JoinHandle;
use std::time::Duration;

pub struct ReloadSettings {
    pub config_file_paths: Vec<String>,
    pub debounce: Duration,
    pub truncate_on_config_change: bool,
    pub current_socket: Option<String>,
}

pub struct ReloadSession {
    ready: Option<mpsc::Receiver<()>>,
    thread: JoinHandle<Result<(), String>>,
}

impl ReloadSession {
    pub fn start(
        settings: ReloadSettings,
        watches: &Watches,
        coordinator: ReloadCoordinator,
        shutdown: Arc<ShutdownCoordinator>,
    ) -> Self {
        let ReloadSettings {
            config_file_paths,
            debounce,
            truncate_on_config_change,
            current_socket,
        } = settings;
        let baselines: std::collections::HashMap<String, std::time::SystemTime> = config_file_paths
            .iter()
            .filter_map(|path| {
                std::fs::metadata(path)
                    .and_then(|metadata| metadata.modified())
                    .ok()
                    .map(|modified| (path.clone(), modified))
            })
            .collect();
        let startup_config_paths = config_file_paths.clone();
        let config_watch_roots = config_watch_roots(&startup_config_paths);
        let reload_config_paths = startup_config_paths.clone();
        let reload_coordinator = coordinator.clone();
        let reload_shutdown = std::sync::Arc::clone(&shutdown);
        let reload_root = watches.root().to_path_buf();
        // TASK-0092: the startup policy doubles as the reload defaults for keys
        // a candidate does not declare. The candidate's OWN declared policy is
        // parsed from its content (see `reload::validate_candidate`), so a
        // concurrency/debounce/backend/gitignore/hooks/socket change is a real
        // semantic change and is applied at commit.
        let reload_defaults = crate::reload::PolicyDefaults {
            concurrency: watches.concurrency(),
            debounce,
            backend: watches.backend(),
            gitignore: watches.respects_gitignore(),
            hooks: watches.hooks(),
            session_hooks: watches.session_hooks(),
        };
        // AC8: the current control socket path (as configured at startup); the
        // reload thread detects candidate path changes and requests a
        // bind-new-before-retire-old handoff through the coordinator.
        let reload_current_socket = current_socket;
        let initial_revision = watches.revision().cloned();
        // TASK-0090: the reload watcher signals readiness after registering its
        // config-path roots; the main loop gates init on it so a config-touching
        // init task never fires before the reload watcher is subscribed.
        let (reload_ready_tx, reload_ready_rx) = std::sync::mpsc::channel();
        let reload_shutdown_flag = shutdown.requested_flag();
        let thread = std::thread::spawn(move || {
            let baselines = std::sync::Mutex::new(baselines);
            let backend = crate::watcher::WatchBackend::Auto;
            let tracker = std::sync::Mutex::new(crate::config_revision::RevisionTracker::new());
            // Seed the reload tracker with the initial revision the composition
            // root already observed, so reload revision numbers continue
            // monotonically from startup (never two trackers disagreeing).
            if let Some(initial) = initial_revision {
                tracker.lock().unwrap().seed(initial);
            }
            let reload_ready_tx = reload_ready_tx;
            let reload_config_paths = reload_config_paths;
            let reload_current_socket = reload_current_socket;
            watcher::events(
                config_watch_roots,
                move || {
                    let _ = reload_ready_tx.send(());
                },
                move |_batch_id: u64, events: &[watcher::FileEvent]| {
                    // AC9: only events targeting the canonical config paths (or
                    // their parents' watched subtrees) trigger validation. Atomic
                    // editor saves surface as a change on the config filename
                    // under the watched parent; unrelated files never validate.
                    let file_changed = changed_config_path(events, &reload_config_paths);
                    if file_changed.is_empty() {
                        return;
                    }

                    // Ignore events that do not reflect a real modification since
                    // the watcher started (historical FSEvents replays).
                    let mut baselines = baselines.lock().unwrap();
                    let current = std::fs::metadata(&file_changed)
                        .and_then(|metadata| metadata.modified())
                        .ok();
                    let baseline = baselines.get(&file_changed).copied();
                    let changed = match (current, baseline) {
                        (Some(current), Some(baseline)) => current != baseline,
                        // Unknown path or missing metadata: treat as real.
                        _ => true,
                    };
                    if !changed {
                        return;
                    }
                    if let Some(current) = current {
                        baselines.insert(file_changed.clone(), current);
                    }
                    drop(baselines);

                    // Contract §2: read the candidate only after the window
                    // settles; a partial write fails validation instead of being
                    // misclassified.
                    let content = match std::fs::read_to_string(&file_changed) {
                        Ok(content) => content,
                        Err(err) => {
                            // Config deleted/renamed: treat as invalid (contract
                            // §7) — the watcher cannot run without a config.
                            fatal_reload(
                                &reload_coordinator,
                                &reload_shutdown,
                                &format!("config unreadable after change: {err}"),
                            );
                            return;
                        }
                    };

                    // AC8: parse the candidate's control socket path up front so
                    // it participates in the semantic decision (a socket move is
                    // a real revision change, never a no-op).
                    let candidate_socket = config::control_socket_from_yaml(&content)
                        .unwrap_or_else(|err| {
                            stdout::warn(&format!("Cannot read socket from candidate: {err}"));
                            None
                        })
                        .map(std::path::PathBuf::from);

                    match crate::reload::decide(
                        &mut tracker.lock().unwrap(),
                        &content,
                        reload_root.clone(),
                        &reload_defaults,
                    ) {
                        crate::reload::ReloadDecision::NoOp => {
                            stdout::info("Config save has no semantic change; nothing to reload.");
                        }
                        crate::reload::ReloadDecision::Commit(revision) => {
                            // TASK-0091 AC3: the reload lifecycle transitions
                            // only when a candidate actually commits (never for a
                            // no-op save): `configReloading` before prepare,
                            // `configReloaded` after the commit boundary.
                            reload_coordinator.lifecycle().reloading(Some(&revision));
                            let candidate_watches = build_watches_from_content(
                                &content,
                                &reload_root,
                                &reload_defaults,
                                revision.clone(),
                            );
                            match candidate_watches {
                                Ok(candidate) => {
                                    let log_sink = |msg: &str| stdout::warn(msg);
                                    // AC8: if the candidate changes the control
                                    // socket path, bind the NEW socket before
                                    // commit (failure is fatal — never a silent
                                    // stale socket) and retire the OLD one after.
                                    let socket_changed = match (
                                        reload_current_socket.as_deref(),
                                        candidate_socket.as_deref(),
                                    ) {
                                        (Some(current), Some(candidate)) => current != candidate,
                                        (None, Some(_)) | (Some(_), None) => true,
                                        (None, None) => false,
                                    };
                                    if socket_changed {
                                        if let Some(new_path) = candidate_socket.as_deref() {
                                            if let Err(err) =
                                                reload_coordinator.prepare_socket(new_path)
                                            {
                                                fatal_reload(
                                                    &reload_coordinator,
                                                    &reload_shutdown,
                                                    &format!("control socket rebind failed: {err}"),
                                                );
                                                return;
                                            }
                                        }
                                    }
                                    // Prepare→commit→retire (contract §4): added
                                    // roots register on the live backend BEFORE
                                    // the pointer swap; any prepare failure takes
                                    // the invalid fatal path with nothing mutated.
                                    let transaction = match reload_coordinator.begin(
                                        revision.clone(),
                                        candidate,
                                        &log_sink,
                                        candidate_socket.clone(),
                                    ) {
                                        Ok(transaction) => transaction,
                                        Err(err) => {
                                            fatal_reload(
                                                &reload_coordinator,
                                                &reload_shutdown,
                                                &format!("reload prepare failed: {err}"),
                                            );
                                            return;
                                        }
                                    };
                                    if let Err(err) = reload_coordinator.commit(&transaction) {
                                        fatal_reload(
                                            &reload_coordinator,
                                            &reload_shutdown,
                                            &format!("reload commit failed: {err}"),
                                        );
                                        return;
                                    }
                                    // TASK-0101: only the successful commit
                                    // replaces the future watcher close hook.
                                    reload_shutdown
                                        .update_hooks(transaction.candidate.session_hooks());
                                    // AC10: truncate-on-change fires only after a
                                    // committed valid semantic reload, preserving
                                    // the deterministic notice order (truncate
                                    // notice precedes the reload notice).
                                    if truncate_on_config_change {
                                        match logging::truncate() {
                                        Ok(()) => stdout::info(
                                            "Log file truncated before reloading configuration.",
                                        ),
                                        Err(err) => stdout::warn(&format!(
                                            "Failed to truncate log file: {err}"
                                        )),
                                    }
                                    }
                                    // Obsolete roots/backend resources retire only
                                    // after the commit boundary (contract §4).
                                    if let Err(err) =
                                        reload_coordinator.retire(&transaction, &log_sink)
                                    {
                                        stdout::warn(&format!("reload retire warning: {err}"));
                                    }
                                    // AC8: retire the OLD control socket after the
                                    // boundary; its file is removed by the server
                                    // drop, and the new socket is already live.
                                    reload_coordinator.retire_socket();
                                    reload_shutdown.set_cleanup_paths(
                                        reload_coordinator.socket_paths_to_cleanup(),
                                    );
                                    // The commit (shared config swap + worker
                                    // revision + backend root swap) completed;
                                    // only now is the reload observable (contract
                                    // §4 live point = atomic commit).
                                    stdout::info(&format!(
                                        "Config change is valid; hot-reloading to revision {}.",
                                        revision.number
                                    ));
                                    // The commit boundary passed; the new revision
                                    // is live and observable.
                                    reload_coordinator.lifecycle().reloaded(&revision);
                                }
                                Err(err) => {
                                    fatal_reload(&reload_coordinator, &reload_shutdown, &err)
                                }
                            }
                        }
                        crate::reload::ReloadDecision::Fatal(error) => {
                            fatal_reload(
                                &reload_coordinator,
                                &reload_shutdown,
                                &format!(
                                    "invalid config ({}): {}",
                                    match error.gate {
                                        crate::reload::ValidationGate::Syntactic => "syntax",
                                        crate::reload::ValidationGate::Schema => "schema",
                                        crate::reload::ValidationGate::Semantic => "semantics",
                                        crate::reload::ValidationGate::Operational => "operational",
                                    },
                                    error.reason
                                ),
                            );
                        }
                    }
                },
                debounce,
                backend,
                false,
                None,
                Some(reload_shutdown_flag),
            )
        });

        Self {
            ready: Some(reload_ready_rx),
            thread,
        }
    }

    pub fn take_ready(&mut self) -> mpsc::Receiver<()> {
        self.ready
            .take()
            .expect("reload readiness receiver taken once")
    }

    pub fn join(self) {
        let _ = self
            .thread
            .join()
            .expect("Failed to join config watcher thread");
    }
}

fn config_watch_roots(config_paths: &[String]) -> Vec<String> {
    let mut parents: Vec<PathBuf> = config_paths
        .iter()
        .filter_map(|path| {
            std::path::Path::new(path)
                .parent()
                .filter(|parent| !parent.as_os_str().is_empty())
                .map(|parent| parent.to_path_buf())
        })
        .collect();
    parents.sort();
    parents.dedup();
    parents
        .into_iter()
        .map(|path| path.display().to_string())
        .collect()
}

fn changed_config_path(events: &[watcher::FileEvent], config_paths: &[String]) -> String {
    events
        .iter()
        .map(|event| event.path.clone())
        .find(|path| {
            config_paths.iter().any(|candidate| {
                path == candidate
                    || path.ends_with(candidate)
                    || std::path::Path::new(path)
                        .canonicalize()
                        .ok()
                        .map(|path| path.to_string_lossy().to_string())
                        .as_deref()
                        == Some(candidate.as_str())
            })
        })
        .unwrap_or_default()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn watch_roots_are_parent_directories_in_stable_order() {
        let paths = vec![
            "/workspace/z/.watch.yaml".to_owned(),
            "/workspace/a/.watch.yml".to_owned(),
            "/workspace/z/other.yaml".to_owned(),
        ];

        assert_eq!(
            config_watch_roots(&paths),
            vec!["/workspace/a".to_owned(), "/workspace/z".to_owned()]
        );
    }

    #[test]
    fn changed_path_selects_only_configured_file() {
        let events = vec![
            watcher::FileEvent {
                path: "/workspace/src/main.rs".to_owned(),
                continuous: false,
            },
            watcher::FileEvent {
                path: "/workspace/.watch.yaml".to_owned(),
                continuous: false,
            },
        ];

        assert_eq!(
            changed_config_path(&events, &["/workspace/.watch.yaml".to_owned()]),
            "/workspace/.watch.yaml"
        );
    }

    #[test]
    fn changed_path_is_empty_for_unrelated_events() {
        let events = vec![watcher::FileEvent {
            path: "/workspace/src/main.rs".to_owned(),
            continuous: false,
        }];

        assert!(changed_config_path(&events, &["/workspace/.watch.yaml".to_owned()]).is_empty());
    }
}

/// Graceful fatal config shutdown (contract §5): emit the terminal error,
/// publish the terminal `configInvalid` lifecycle transition so control
/// subscribers observe it, reap owned children/services through process
/// ownership, remove the control socket file(s) explicitly (`process::exit`
/// skips the ControlServer Drop), and exit nonzero. Never
/// SIGKILL/panic/self-SIGTERM.
fn fatal_reload(
    coordinator: &crate::reload_coordinator::ReloadCoordinator,
    shutdown: &crate::shutdown::ShutdownCoordinator,
    reason: &str,
) {
    let current = coordinator.current();
    stdout::error(&format!(
        "Fatal configuration error; terminating watcher.\nWorkspace: {}\nReason: {}",
        current.root().display(),
        reason
    ));
    // TASK-0091 AC8: publish the terminal config diagnostic BEFORE the
    // socket closes — subscribers observe `configInvalid`, then disconnect.
    // Best effort: the process exits right after, so a slow subscriber may
    // miss the notification (bounded, "when possible").
    coordinator
        .lifecycle()
        .invalid(current.revision(), reason.to_owned());
    // TASK-0101: freeze the first fatal reason and last successfully
    // committed close hook. The normal watch thread owns reaping, resource
    // cleanup, hook execution, and the final exit — never this reload thread.
    shutdown.set_cleanup_paths(coordinator.socket_paths_to_cleanup());
    shutdown.request(crate::shutdown::ShutdownReason::FatalConfig {
        detail: reason.to_owned(),
        exit_code: 1,
    });
}

/// Builds a fresh `Watches` from a validated config candidate, bound to the
/// new revision (TASK-0090 commit). The candidate's OWN declared policy
/// (concurrency/debounce/backend/gitignore/hooks) is parsed from the content;
/// missing keys keep the startup defaults — so a policy change committed by
/// the reload is actually applied to post-commit generations (TASK-0092).
fn build_watches_from_content(
    content: &str,
    root: &std::path::Path,
    defaults: &crate::reload::PolicyDefaults,
    revision: crate::config_revision::ConfigRevision,
) -> Result<Watches, String> {
    let rules = crate::config::from_yaml(content).map_err(|err| err.to_string())?;
    crate::rules::validate_rules(&rules).map_err(|err| err.to_string())?;
    let concurrency = crate::config::concurrency_from_yaml(content)
        .map_err(|err| err.to_string())?
        .unwrap_or(defaults.concurrency);
    let debounce = crate::config::debounce_from_yaml(content)
        .map_err(|err| err.to_string())?
        .unwrap_or(defaults.debounce);
    let backend = crate::config::watch_backend_from_yaml(content)
        .map_err(|err| err.to_string())?
        .unwrap_or(defaults.backend.clone());
    let respect_gitignore =
        crate::config::respect_gitignore_from_yaml(content).map_err(|err| err.to_string())?;
    let hooks =
        crate::config::generation_hooks_from_yaml(content).map_err(|err| err.to_string())?;
    let session_hooks =
        crate::config::session_hooks_from_yaml(content).map_err(|err| err.to_string())?;
    Ok(
        Watches::with_root_and_concurrency(rules, root.to_path_buf(), concurrency)
            .with_debounce(debounce)
            .with_backend(backend)
            .with_gitignore(respect_gitignore)
            .with_hooks(hooks)
            .with_session_hooks(session_hooks)
            .with_revision(revision),
    )
}
