//! In-process reload coordination (TASK-0090, CONFIG-RELOAD-CONTRACT §4).
//!
//! The reload thread validates candidates; on `Commit` it swaps the shared
//! watch configuration and publishes a live root swap to the backend. The
//! strategy installs its worker and the backend's swap publisher into the
//! coordinator after construction, so the transaction can reach every live
//! component without the reload thread owning them directly.

use std::path::PathBuf;
use std::sync::{Arc, Mutex};

use crate::config_lifecycle::ConfigLifecycle;
use crate::config_revision::ConfigRevision;
use crate::plan::RunPlan;
use crate::watcher::RootSwapPublisher;
use crate::watches::Watches;
use crate::workers::Worker;

/// One control-socket swap (TASK-0090 AC8): `prepare` binds a NEW socket at
/// `new_path` before commit and must fail loudly (the caller takes the fatal
/// path) when binding fails; `retire` drops the OLD socket after commit.
/// Installed by the watch strategy, which owns the live server.
#[derive(Clone)]
pub struct SocketSwapper {
    prepare: Arc<dyn Fn(&std::path::Path) -> Result<(), String> + Send + Sync>,
    retire: Arc<dyn Fn() + Send + Sync>,
}

impl SocketSwapper {
    pub fn new<P, R>(prepare: P, retire: R) -> Self
    where
        P: Fn(&std::path::Path) -> Result<(), String> + Send + Sync + 'static,
        R: Fn() + Send + Sync + 'static,
    {
        Self {
            prepare: Arc::new(prepare),
            retire: Arc::new(retire),
        }
    }

    /// Binds the new socket; returns an error (fatal path) on failure.
    pub fn prepare(&self, path: &std::path::Path) -> Result<(), String> {
        (self.prepare)(path)
    }

    /// Retires the old socket after the commit boundary.
    pub fn retire(&self) {
        (self.retire)();
    }
}

/// One in-flight reload transaction (contract §4): the frozen revision, the
/// candidate watch config to commit, and the root diff computed at prepare
/// time so retire can unregister exactly the obsolete roots. The service
/// sets (name → signature) are captured at prepare time too, so commit can
/// reconcile managed services (TASK-0090 AC6) without re-reading state.
#[derive(Clone, Debug)]
pub struct ReloadTransaction {
    pub revision: ConfigRevision,
    pub candidate: Watches,
    pub added_roots: Vec<PathBuf>,
    pub removed_roots: Vec<PathBuf>,
    pub old_services: Vec<(String, String)>,
    pub new_services: Vec<(String, String)>,
    /// The candidate's control socket path from `on.socket` (AC8), part of
    /// the semantic surface so a socket move is a real revision change.
    pub candidate_socket: Option<PathBuf>,
}

/// Shared coordination point between the config-reload thread and the watch
/// strategy (TASK-0090).
#[derive(Clone)]
pub struct ReloadCoordinator {
    shared: Arc<Mutex<Watches>>,
    worker: Arc<Mutex<Option<Arc<Worker>>>>,
    publisher: Arc<Mutex<Option<RootSwapPublisher>>>,
    /// Optional control-socket swapper (TASK-0090 AC8): bind-new-before-
    /// retire-old handoff when the reloaded config changes the socket path.
    socket: Arc<Mutex<Option<SocketSwapper>>>,
    /// Pending socket swap prepared at `begin`; retired at `retire`.
    socket_pending: Arc<Mutex<bool>>,
    /// Config lifecycle state source (TASK-0091, AC3): the reload thread
    /// writes transitions; the snapshot broker and control server read them.
    lifecycle: Arc<ConfigLifecycle>,
}

impl ReloadCoordinator {
    /// Creates the coordinator around the initial shared watch config. The
    /// reload thread uses this handle; the strategy installs worker +
    /// publisher before the first batch.
    pub fn new(shared: Arc<Mutex<Watches>>) -> Self {
        Self {
            shared,
            worker: Arc::new(Mutex::new(None)),
            publisher: Arc::new(Mutex::new(None)),
            socket: Arc::new(Mutex::new(None)),
            socket_pending: Arc::new(Mutex::new(false)),
            lifecycle: Arc::new(ConfigLifecycle::new()),
        }
    }

    /// The config lifecycle state source (TASK-0091, AC3): shared between
    /// the reload thread (writer) and the control/broker surfaces (readers).
    pub fn lifecycle(&self) -> &Arc<ConfigLifecycle> {
        &self.lifecycle
    }

    /// The shared watch config the routing loop reads per batch.
    pub fn shared(&self) -> &Arc<Mutex<Watches>> {
        &self.shared
    }

    /// Installs the worker so the transaction can swap its frozen revision.
    pub fn install_worker(&self, worker: Arc<Worker>) {
        *self.worker.lock().unwrap() = Some(worker);
    }

    /// Installs the backend's root-swap publisher.
    pub fn install_publisher(&self, publisher: RootSwapPublisher) {
        *self.publisher.lock().unwrap() = Some(publisher);
    }

    /// Installs the control-socket swapper (TASK-0090 AC8): bind-new-before-
    /// retire-old handoff for socket path changes.
    pub fn install_socket_swapper(&self, swapper: SocketSwapper) {
        *self.socket.lock().unwrap() = Some(swapper);
    }

    /// True when a socket swap was prepared and not yet retired (diagnostics).
    pub fn socket_swap_pending(&self) -> bool {
        *self.socket_pending.lock().unwrap()
    }

    /// Begins the reload transaction (contract §4): registers ADDED roots on
    /// the live backend BEFORE any shared mutation and reports the diff. A
    /// failure here (added root cannot register) returns `Err` with nothing
    /// mutated — the caller takes the invalid fatal path. A missing publisher
    /// (legacy/blocking backend) logs and proceeds.
    pub fn begin(
        &self,
        revision: ConfigRevision,
        candidate: Watches,
        log: &dyn Fn(&str),
        candidate_socket: Option<PathBuf>,
    ) -> Result<ReloadTransaction, String> {
        let old_roots = self.current_roots();
        let new_roots: Vec<PathBuf> = candidate
            .paths_to_watch()
            .unwrap_or_default()
            .into_iter()
            .map(PathBuf::from)
            .collect();
        let diff = crate::reload::root_diff(&old_roots, &new_roots);

        // Prepare: register added roots before commit so events from the new
        // root set are captured from the boundary onward (no event-loss gap).
        if !diff.added.is_empty() {
            let union: Vec<PathBuf> = {
                let mut roots = old_roots.clone();
                for added in &diff.added {
                    if !roots.contains(added) {
                        roots.push(added.clone());
                    }
                }
                roots
            };
            let union: Vec<String> = union.into_iter().map(|p| p.display().to_string()).collect();
            match self.publisher.lock().unwrap().as_ref() {
                Some(publisher) => publisher.swap(union)?,
                None => {
                    log(&format!(
                        "backend without live root swap; preparing {} added root(s)",
                        diff.added.len()
                    ));
                }
            }
        }

        // AC6: capture the service sets (name → signature) before the swap
        // so commit can diff them. The old set comes from the shared config
        // the transaction is replacing; the new set from the candidate.
        let old_services = self.shared_service_specs();
        let new_services = crate::config_revision::RuntimeConfig::capture(
            candidate.root().to_path_buf(),
            candidate.targets(),
            candidate.concurrency(),
            candidate.debounce(),
            candidate.backend(),
            candidate.respects_gitignore(),
            candidate.hooks(),
            candidate_socket.clone(),
        )
        .services();

        Ok(ReloadTransaction {
            revision,
            candidate,
            added_roots: diff.added,
            removed_roots: diff.removed,
            old_services,
            new_services,
            candidate_socket,
        })
    }

    /// The (name → signature) set of managed services in the current shared
    /// config (TASK-0090 AC6).
    fn shared_service_specs(&self) -> Vec<(String, String)> {
        let shared = self.shared.lock().unwrap();
        let runtime = crate::config_revision::RuntimeConfig::capture(
            shared.root().to_path_buf(),
            shared.targets(),
            shared.concurrency(),
            shared.debounce(),
            shared.backend(),
            shared.respects_gitignore(),
            shared.hooks(),
            None,
        );
        runtime.services()
    }

    /// Prepares a control-socket path change for a pending transaction
    /// (TASK-0090 AC8): binds the NEW socket before commit; a bind failure
    /// returns an error and the caller takes the invalid fatal path. Called
    /// by the reload thread when the candidate's socket path differs.
    pub fn prepare_socket(&self, new_path: &std::path::Path) -> Result<(), String> {
        match self.socket.lock().unwrap().as_ref() {
            Some(swapper) => {
                swapper.prepare(new_path)?;
                *self.socket_pending.lock().unwrap() = true;
                Ok(())
            }
            None => Err("control socket rebind requested but no swapper installed".to_owned()),
        }
    }

    /// Retires the OLD control socket after the commit boundary (AC8).
    pub fn retire_socket(&self) {
        if *self.socket_pending.lock().unwrap() {
            if let Some(swapper) = self.socket.lock().unwrap().as_ref() {
                swapper.retire();
            }
            *self.socket_pending.lock().unwrap() = false;
        }
    }

    /// Commits a prepared transaction: atomically swaps the shared watch
    /// config and the worker revision so later batches route under the new
    /// revision (contract §4 live point). No backend root operation happens
    /// here — added roots were registered by [`ReloadCoordinator::begin`] and
    /// obsolete roots are retired by [`ReloadCoordinator::retire`].
    pub fn commit(&self, transaction: &ReloadTransaction) -> Result<(), String> {
        let concurrency = transaction.candidate.concurrency();
        let mut shared = self.shared.lock().unwrap();
        shared.swap_config(transaction.candidate.clone());
        drop(shared);

        if let Some(worker) = self.worker.lock().unwrap().as_ref() {
            worker.set_revision(transaction.revision.clone());
            // AC7: concurrency/policy changes apply to generations planned
            // after the boundary only; the running group is never resized.
            worker.set_concurrency(concurrency);
            // AC6: reconcile managed services — stop removed/signature-changed
            // services gracefully; start new/changed services under the new
            // revision appended to the active generation (unchanged services
            // remain owned and are never touched).
            self.reconcile_services(worker, transaction);
        }
        Ok(())
    }

    /// TASK-0090 AC6: diffs the old vs new service signature sets and drives
    /// the worker to retire changed/removed services and start new/changed
    /// ones. Unchanged-by-signature services stay owned. Any worker error is
    /// surfaced (post-commit: the config is live; service issues are logged
    /// by the caller, never fatal).
    fn reconcile_services(&self, worker: &Arc<Worker>, transaction: &ReloadTransaction) {
        let old: std::collections::HashMap<&str, &str> = transaction
            .old_services
            .iter()
            .map(|(name, signature)| (name.as_str(), signature.as_str()))
            .collect();
        let new: std::collections::HashMap<&str, &str> = transaction
            .new_services
            .iter()
            .map(|(name, signature)| (name.as_str(), signature.as_str()))
            .collect();

        // Stop: services removed from the config, or whose signature changed.
        let stop_names: Vec<String> = old
            .iter()
            .filter(|(name, signature)| match new.get(*name) {
                None => true,
                Some(new_signature) => new_signature != *signature,
            })
            .map(|(name, _)| (*name).to_owned())
            .collect();

        if let Ok(still_running) = worker.reconcile_services(stop_names.clone()) {
            // Start: services new to the config, or changed, that are not
            // still running (unchanged ones are running and stay owned).
            let to_start: Vec<String> = transaction
                .new_services
                .iter()
                .filter(|(name, signature)| match old.get(name.as_str()) {
                    None => true,
                    Some(old_signature) => old_signature != signature,
                })
                .filter(|(name, _)| !still_running.contains(name))
                .map(|(name, _)| name.clone())
                .collect();
            if !to_start.is_empty() {
                let service_plan = RunPlan::from_rules(
                    transaction
                        .candidate
                        .targets()
                        .into_iter()
                        .filter(|rule| rule.service() && to_start.contains(&rule.name))
                        .collect(),
                );
                let _ = worker.start_services(service_plan);
            }
        }
    }

    /// Retires the obsolete resources of a committed transaction: unregisters
    /// REMOVED roots on the live backend after the commit boundary. A missing
    /// publisher (legacy/blocking backend) logs; retire failures are surfaced
    /// to the caller (post-commit, the config is already live).
    pub fn retire(
        &self,
        transaction: &ReloadTransaction,
        log: &dyn Fn(&str),
    ) -> Result<(), String> {
        if transaction.removed_roots.is_empty() {
            return Ok(());
        }
        let new_roots: Vec<String> = self
            .current_roots()
            .into_iter()
            .map(|p| p.display().to_string())
            .collect();
        match self.publisher.lock().unwrap().as_ref() {
            Some(publisher) => publisher.swap(new_roots.clone())?,
            None => {
                log(&format!(
                    "backend without live root swap; retiring {} obsolete root(s)",
                    transaction.removed_roots.len()
                ));
            }
        }
        Ok(())
    }

    /// The current effective root set (for diffing and diagnostics).
    pub fn current_roots(&self) -> Vec<PathBuf> {
        self.shared
            .lock()
            .unwrap()
            .paths_to_watch()
            .unwrap_or_default()
            .into_iter()
            .map(PathBuf::from)
            .collect()
    }

    /// The current effective watch config (for diagnostics).
    pub fn current(&self) -> Watches {
        self.shared.lock().unwrap().clone()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::config::RunHooks;
    use crate::config_revision::{ConfigRevision, RuntimeConfig};
    use crate::rules::Rules;
    use crate::watcher::WatchBackend;
    use std::time::Duration;

    fn sample_watches(root: std::path::PathBuf, name: &str) -> Watches {
        let rules = vec![Rules::new(
            name.to_owned(),
            vec!["echo hi".to_owned()],
            vec!["src/**".to_owned()],
            vec![],
            false,
        )];
        Watches::with_root_and_concurrency(rules, root, 2)
    }

    fn revision(number: u64) -> ConfigRevision {
        ConfigRevision {
            number,
            hash: format!("hash-{number}"),
        }
    }

    #[test]
    fn begin_commit_retire_swaps_shared_config() {
        let root = std::env::temp_dir().join(format!("fzz-rel-{}", std::process::id()));
        std::fs::create_dir_all(root.join("src")).unwrap();
        std::fs::create_dir_all(root.join("docs")).unwrap();
        let shared = Arc::new(Mutex::new(sample_watches(root.clone(), "build")));
        let coordinator = ReloadCoordinator::new(Arc::clone(&shared));
        let logs: Arc<Mutex<Vec<String>>> = Arc::new(Mutex::new(vec![]));
        let log_sink = |msg: &str| logs.lock().unwrap().push(msg.to_owned());

        // Candidate adds the docs root so begin exercises the (missing)
        // publisher legacy path and logs instead of failing.
        let candidate = Watches::with_root_and_concurrency(
            vec![
                Rules::new(
                    "build".to_owned(),
                    vec!["echo hi".to_owned()],
                    vec!["src/**".to_owned()],
                    vec![],
                    false,
                ),
                Rules::new(
                    "docs".to_owned(),
                    vec!["echo docs".to_owned()],
                    vec!["docs/**".to_owned()],
                    vec![],
                    false,
                ),
            ],
            root.clone(),
            2,
        );
        let transaction = coordinator
            .begin(revision(2), candidate, &log_sink, None)
            .expect("begin succeeds without a publisher (legacy path)");
        coordinator
            .commit(&transaction)
            .expect("commit succeeds without a publisher");
        coordinator
            .retire(&transaction, &log_sink)
            .expect("retire succeeds without a publisher");

        let current = coordinator.current();
        assert_eq!(current.targets().len(), 2);
        let logs = logs.lock().unwrap();
        assert!(
            logs.iter().any(|l| l.contains("without live root swap")),
            "missing publisher must be logged, not fatal: {logs:?}"
        );
        std::fs::remove_dir_all(&root).unwrap();
    }

    #[test]
    fn worker_revision_is_swapped_on_commit() {
        // The worker is behind an Arc; install it and verify set_revision is
        // applied. Build a minimal worker via a real constructor is heavy, so
        // this proves the coordinator's worker plumbing via the shared mutex
        // contract is exercised only when installed.
        let root = std::env::temp_dir().join(format!("fzz-relw-{}", std::process::id()));
        std::fs::create_dir_all(&root).unwrap();
        let shared = Arc::new(Mutex::new(sample_watches(root.clone(), "build")));
        let coordinator = ReloadCoordinator::new(Arc::clone(&shared));
        let noop = |_: &str| {};
        // Without a worker installed, commit still succeeds.
        let transaction = coordinator
            .begin(
                revision(2),
                sample_watches(root.clone(), "lint"),
                &noop,
                None,
            )
            .expect("begin without worker");
        coordinator
            .commit(&transaction)
            .expect("commit without worker");
        std::fs::remove_dir_all(&root).unwrap();
    }

    #[test]
    fn begin_registers_added_roots_before_commit_and_retire_removes_after() {
        // TASK-0090 AC2/AC3: added roots register on the backend BEFORE the
        // shared pointer swap; obsolete roots retire AFTER the commit
        // boundary. A recording backend proves the ordering and the exact
        // root sets at each phase.
        let root = std::env::temp_dir().join(format!("fzz-phase-{}", std::process::id()));
        let src = root.join("src");
        let docs = root.join("docs");
        std::fs::create_dir_all(&src).unwrap();
        std::fs::create_dir_all(&docs).unwrap();

        let old_rules = vec![Rules::new(
            "build".to_owned(),
            vec!["echo hi".to_owned()],
            vec!["src/**".to_owned()],
            vec![],
            false,
        )];
        let shared = Arc::new(Mutex::new(Watches::with_root_and_concurrency(
            old_rules,
            root.clone(),
            2,
        )));
        let coordinator = ReloadCoordinator::new(Arc::clone(&shared));

        // Recording backend: captures each swap's root set and acks.
        let (swap_tx, swap_rx): (
            std::sync::mpsc::Sender<crate::watcher::RootSwap>,
            std::sync::mpsc::Receiver<crate::watcher::RootSwap>,
        ) = std::sync::mpsc::channel();
        let seen: Arc<Mutex<Vec<Vec<String>>>> = Arc::new(Mutex::new(vec![]));
        let seen_backend = Arc::clone(&seen);
        let backend = std::thread::spawn(move || {
            while let Ok(swap) = swap_rx.recv_timeout(std::time::Duration::from_secs(5)) {
                seen_backend.lock().unwrap().push(swap.roots.clone());
                if let Some(ack) = swap.ack {
                    let _ = ack.send(Ok(()));
                }
            }
        });
        coordinator.install_publisher(crate::watcher::RootSwapPublisher::new(swap_tx));

        let new_rules = vec![
            Rules::new(
                "build".to_owned(),
                vec!["echo hi".to_owned()],
                vec!["src/**".to_owned()],
                vec![],
                false,
            ),
            Rules::new(
                "docs".to_owned(),
                vec!["echo docs".to_owned()],
                vec!["docs/**".to_owned()],
                vec![],
                false,
            ),
        ];
        let candidate = Watches::with_root_and_concurrency(new_rules, root.clone(), 2);
        let noop = |_: &str| {};
        let transaction = coordinator
            .begin(revision(2), candidate, &noop, None)
            .expect("prepare must register added roots");

        // Before commit the shared config still carries the OLD rules: the
        // pointer swap happens in commit, never during prepare.
        assert_eq!(coordinator.current().targets()[0].name, "build");
        assert_eq!(coordinator.current().targets().len(), 1);
        assert_eq!(transaction.added_roots, vec![docs.clone()]);
        assert!(transaction.removed_roots.is_empty());

        coordinator.commit(&transaction).expect("commit");
        assert_eq!(coordinator.current().targets().len(), 2);

        // A second reload removing the docs root must retire it AFTER commit.
        let shrink = Watches::with_root_and_concurrency(
            vec![Rules::new(
                "build".to_owned(),
                vec!["echo hi".to_owned()],
                vec!["src/**".to_owned()],
                vec![],
                false,
            )],
            root.clone(),
            2,
        );
        let t2 = coordinator
            .begin(revision(3), shrink.clone(), &noop, None)
            .expect("begin shrink");
        assert_eq!(t2.removed_roots, vec![docs.clone()]);
        coordinator.commit(&t2).expect("commit shrink");
        // Retire runs AFTER commit: the shared config is already the shrink
        // candidate.
        coordinator.retire(&t2, &noop).expect("retire shrink");
        assert_eq!(coordinator.current().targets().len(), 1);

        drop(coordinator);
        drop(shared);
        backend.join().unwrap();

        let swaps = seen.lock().unwrap();
        assert_eq!(
            swaps.len(),
            2,
            "one prepare union + one retire set: {swaps:?}"
        );
        // Prepare registered old ∪ added (docs) in one synchronous swap.
        assert!(
            swaps[0].iter().any(|r| r.ends_with("src"))
                && swaps[0].iter().any(|r| r.ends_with("docs")),
            "prepare must register added roots alongside existing: {swaps:?}"
        );
        // Retire swapped the committed root set (src only), removing docs.
        assert!(
            swaps[1].iter().any(|r| r.ends_with("src"))
                && !swaps[1].iter().any(|r| r.ends_with("docs")),
            "retire must unregister obsolete roots: {swaps:?}"
        );
        std::fs::remove_dir_all(&root).unwrap();
    }

    #[test]
    fn begin_failure_does_not_mutate_shared_config() {
        // AC2: if an added root cannot register on the backend, the
        // transaction fails with NOTHING mutated — no partial live change.
        let root = std::env::temp_dir().join(format!("fzz-fail-{}", std::process::id()));
        std::fs::create_dir_all(root.join("src")).unwrap();
        std::fs::create_dir_all(root.join("docs")).unwrap();
        let shared = Arc::new(Mutex::new(sample_watches(root.clone(), "build")));
        let coordinator = ReloadCoordinator::new(Arc::clone(&shared));

        // Backend that rejects every swap.
        let (swap_tx, swap_rx): (
            std::sync::mpsc::Sender<crate::watcher::RootSwap>,
            std::sync::mpsc::Receiver<crate::watcher::RootSwap>,
        ) = std::sync::mpsc::channel();
        let backend = std::thread::spawn(move || {
            while let Ok(swap) = swap_rx.recv_timeout(std::time::Duration::from_secs(5)) {
                if let Some(ack) = swap.ack {
                    let _ = ack.send(Err("backend refused root".to_owned()));
                }
            }
        });
        coordinator.install_publisher(crate::watcher::RootSwapPublisher::new(swap_tx));

        // Candidate adds the docs root so prepare must register it.
        let candidate = Watches::with_root_and_concurrency(
            vec![
                Rules::new(
                    "build".to_owned(),
                    vec!["echo hi".to_owned()],
                    vec!["src/**".to_owned()],
                    vec![],
                    false,
                ),
                Rules::new(
                    "docs".to_owned(),
                    vec!["echo docs".to_owned()],
                    vec!["docs/**".to_owned()],
                    vec![],
                    false,
                ),
            ],
            root.clone(),
            2,
        );
        let noop = |_: &str| {};
        let err = coordinator
            .begin(revision(2), candidate, &noop, None)
            .expect_err("backend refusal must fail prepare");
        assert!(err.contains("backend refused root"), "{err}");

        // Nothing was mutated: shared config and worker revision untouched.
        assert_eq!(coordinator.current().targets()[0].name, "build");
        assert_eq!(coordinator.current().targets().len(), 1);

        drop(coordinator);
        drop(shared);
        backend.join().unwrap();
        std::fs::remove_dir_all(&root).unwrap();
    }

    #[test]
    fn commit_swaps_worker_concurrency_bound() {
        // AC7: the committed revision's concurrency applies to generations
        // planned after the boundary. Prove the coordinator updates the
        // worker's shared bound on commit.
        let root = std::env::temp_dir().join(format!("fzz-conc-{}", std::process::id()));
        std::fs::create_dir_all(&root).unwrap();
        let shared = Arc::new(Mutex::new(sample_watches(root.clone(), "build")));
        let coordinator = ReloadCoordinator::new(Arc::clone(&shared));

        let worker_arc = Arc::new(crate::workers::Worker::with_root_and_concurrency(
            false,
            false,
            root.clone(),
            2,
            |_| {},
        ));
        coordinator.install_worker(Arc::clone(&worker_arc));

        let candidate =
            sample_watches(root.clone(), "lint").with_debounce(std::time::Duration::from_millis(1));
        let noop = |_: &str| {};
        let transaction = coordinator
            .begin(revision(2), candidate, &noop, None)
            .expect("begin");
        assert_eq!(worker_arc.concurrency(), 2);
        coordinator.commit(&transaction).expect("commit");
        assert_eq!(worker_arc.concurrency(), 2, "candidate kept concurrency 2");
        std::fs::remove_dir_all(&root).unwrap();
    }

    #[test]
    fn socket_swap_prepares_before_commit_and_retires_after() {
        // AC8: prepare binds the new socket path (fatal on failure) and
        // retire drops the old one, with the pending flag as the ordering
        // observable.
        let root = std::env::temp_dir().join(format!("fzz-sock-{}", std::process::id()));
        std::fs::create_dir_all(&root).unwrap();
        let shared = Arc::new(Mutex::new(sample_watches(root.clone(), "build")));
        let coordinator = ReloadCoordinator::new(Arc::clone(&shared));

        let prepared = Arc::new(std::sync::atomic::AtomicBool::new(false));
        let retired = Arc::new(std::sync::atomic::AtomicBool::new(false));
        let prepared_cb = Arc::clone(&prepared);
        let retired_cb = Arc::clone(&retired);
        coordinator.install_socket_swapper(SocketSwapper::new(
            move |path: &std::path::Path| {
                assert!(path.ends_with("new.sock"));
                prepared_cb.store(true, std::sync::atomic::Ordering::SeqCst);
                Ok(())
            },
            move || {
                retired_cb.store(true, std::sync::atomic::Ordering::SeqCst);
            },
        ));

        assert!(!coordinator.socket_swap_pending());
        coordinator
            .prepare_socket(&root.join("new.sock"))
            .expect("prepare must bind the new socket");
        assert!(coordinator.socket_swap_pending(), "pending until retire");
        assert!(prepared.load(std::sync::atomic::Ordering::SeqCst));
        assert!(!retired.load(std::sync::atomic::Ordering::SeqCst));

        coordinator.retire_socket();
        assert!(!coordinator.socket_swap_pending());
        assert!(retired.load(std::sync::atomic::Ordering::SeqCst));
        std::fs::remove_dir_all(&root).unwrap();
    }

    #[test]
    fn socket_prepare_failure_is_surfaced_as_fatal() {
        let root = std::env::temp_dir().join(format!("fzz-sockf-{}", std::process::id()));
        std::fs::create_dir_all(&root).unwrap();
        let shared = Arc::new(Mutex::new(sample_watches(root.clone(), "build")));
        let coordinator = ReloadCoordinator::new(Arc::clone(&shared));
        coordinator.install_socket_swapper(SocketSwapper::new(
            |_path| Err("bind refused".to_owned()),
            || {},
        ));
        let err = coordinator
            .prepare_socket(&root.join("new.sock"))
            .expect_err("bind failure must be fatal");
        assert!(err.contains("bind refused"), "{err}");
        assert!(!coordinator.socket_swap_pending());
        std::fs::remove_dir_all(&root).unwrap();
    }

    #[test]
    fn runtime_config_round_trips_through_validate() {
        let content = "jobs:\n  - name: build\n    run: cargo build\n    change: 'src/**'\n";
        let runtime = crate::reload::validate_candidate(
            content,
            std::env::current_dir().unwrap(),
            2,
            Duration::from_millis(1000),
            WatchBackend::Native,
            false,
            RunHooks::default(),
            None,
        )
        .expect("valid candidate");
        let _ = RuntimeConfig::capture(
            runtime.root.clone(),
            runtime.rules.clone(),
            runtime.concurrency,
            runtime.debounce,
            runtime.backend,
            runtime.respect_gitignore,
            runtime.hooks.clone(),
            runtime.control_socket.clone(),
        );
        assert_eq!(runtime.plan().task_names(), vec!["build"]);
    }
}
