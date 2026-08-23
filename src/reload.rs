//! Config reload transaction (TASK-0090, CONFIG-RELOAD-CONTRACT §1–§6).
//!
//! In-process reload is a prepare→commit→retire transaction:
//!
//! - **validate** runs the four gates (syntactic, schema, semantic,
//!   operational) against a candidate read after the debounce window;
//! - **prepare** registers every added resource (watch roots) before commit;
//! - **commit** atomically swaps the live runtime config to the new revision;
//! - **retire** removes obsolete roots only after commit.
//!
//! An invalid candidate never publishes a revision or mutates live objects:
//! the caller must take the graceful fatal shutdown path
//! (`process_owner::shutdown_all` + nonzero exit) instead.

use std::path::PathBuf;

use crate::config::{GenerationHooks, SessionHooks};
use crate::config_revision::{ConfigRevision, RevisionTracker, RuntimeConfig};
use crate::rules::Rules;
use crate::watcher::WatchBackend;
use std::time::Duration;

/// The four validation gates (contract §1). `Operational` is reported by the
/// caller after trying to register resources; the first three are pure.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum ValidationGate {
    Syntactic,
    Schema,
    Semantic,
    Operational,
}

/// A fatal config error: the gate that failed plus the reason. Never leaves
/// old config running silently — the caller shuts down gracefully.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ConfigError {
    pub gate: ValidationGate,
    pub reason: String,
}

/// Startup policy values used when a candidate does not declare a key.
/// The candidate's OWN declared policy participates in the revision
/// (CONFIG-RELOAD-CONTRACT §6): a concurrency/debounce/backend/gitignore/
/// hooks/socket change is a real semantic change, never a silent no-op
/// (TASK-0092).
#[derive(Clone, Debug)]
pub struct PolicyDefaults {
    pub concurrency: usize,
    pub debounce: Duration,
    pub backend: WatchBackend,
    pub gitignore: bool,
    pub recovery_policy: crate::config::RecoveryPolicy,
    pub recovery_timeout: Duration,
    pub hooks: GenerationHooks,
    pub session_hooks: SessionHooks,
}

/// Validates a candidate config text against the first three pure gates and
/// builds the frozen runtime config from the candidate's OWN declared policy
/// (missing keys keep the startup defaults). Operational preparation
/// (roots/services) is checked separately by the caller before commit.
pub fn validate_candidate(
    content: &str,
    root: PathBuf,
    defaults: &PolicyDefaults,
) -> Result<RuntimeConfig, ConfigError> {
    // Syntactic gate: parses as YAML documents.
    let rules: Vec<Rules> = crate::config::from_yaml(content).map_err(|err| ConfigError {
        gate: ValidationGate::Syntactic,
        reason: err.to_string(),
    })?;

    // Schema + semantic gates: rule validation (globs, values, coherence).
    crate::rules::validate_rules(&rules).map_err(|err| ConfigError {
        gate: ValidationGate::Semantic,
        reason: err.to_string(),
    })?;

    // TASK-0092: the candidate's policy surface (concurrency, debounce,
    // backend, gitignore, hooks, control socket) is parsed from the
    // candidate itself so a policy change is a real revision change.
    // Missing keys fall back to the startup defaults so omitting them
    // stays a no-op. Parse errors (e.g. `concurrency: 0`) are Semantic.
    let semantic = |reason: String| ConfigError {
        gate: ValidationGate::Semantic,
        reason,
    };
    let concurrency = crate::config::concurrency_from_yaml(content)
        .map_err(semantic)?
        .unwrap_or(defaults.concurrency);
    let debounce = crate::config::debounce_from_yaml(content)
        .map_err(semantic)?
        .unwrap_or(defaults.debounce);
    let backend = crate::config::watch_backend_from_yaml(content)
        .map_err(semantic)?
        .unwrap_or(defaults.backend);
    let respect_gitignore =
        crate::config::respect_gitignore_from_yaml(content).map_err(semantic)?;
    let recovery_policy =
        crate::config::recovery_policy_from_yaml_with_default(content, defaults.recovery_policy)
            .map_err(semantic)?;
    let recovery_timeout =
        crate::config::recovery_timeout_from_yaml_with_default(content, defaults.recovery_timeout)
            .map_err(semantic)?;
    let hooks = crate::config::generation_hooks_from_yaml(content).map_err(semantic)?;
    let session_hooks = crate::config::session_hooks_from_yaml(content).map_err(semantic)?;
    let control_socket = crate::config::control_socket_from_yaml(content)
        .map_err(semantic)?
        .map(std::path::PathBuf::from);

    Ok(RuntimeConfig::capture(
        root,
        rules,
        concurrency,
        debounce,
        backend,
        respect_gitignore,
        recovery_policy,
        recovery_timeout,
        hooks,
        session_hooks,
        control_socket,
    ))
}

/// One complete reload decision after observing a candidate: publish a new
/// revision, report a no-op, or fail fatally.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum ReloadDecision {
    /// Candidate is valid and semantically different; the new revision is
    /// ready to commit (prepare first, then commit).
    Commit(ConfigRevision),
    /// Candidate is valid but semantically identical; nothing changes.
    NoOp,
    /// Candidate failed a gate; the watcher must shut down gracefully.
    Fatal(ConfigError),
}

/// Runs the validate→track flow for one candidate: builds the runtime config
/// (pure gates + candidate-declared policy), then asks the tracker whether it
/// is a semantic change. Operational preparation is NOT part of this
/// decision; the caller prepares roots before committing.
pub fn decide(
    tracker: &mut RevisionTracker,
    content: &str,
    root: PathBuf,
    defaults: &PolicyDefaults,
) -> ReloadDecision {
    match validate_candidate(content, root, defaults) {
        Err(error) => ReloadDecision::Fatal(error),
        Ok(config) => match tracker.observe(&config) {
            crate::config_revision::RevisionDecision::New(revision) => {
                ReloadDecision::Commit(revision)
            }
            crate::config_revision::RevisionDecision::NoOp => ReloadDecision::NoOp,
        },
    }
}

/// Deterministic root-set diff for prepare→commit→retire (contract §4, §6).
/// `added` are roots to register before commit; `removed` are roots to
/// retire after commit. Containment-minimized roots are assumed on input
/// (they come from `Watches::subscription_roots`), so the diff is a plain
/// set difference; ordering is stable.
pub fn root_diff(old_roots: &[PathBuf], new_roots: &[PathBuf]) -> RootDiff {
    RootDiff {
        added: new_roots
            .iter()
            .filter(|root| !old_roots.contains(root))
            .cloned()
            .collect(),
        removed: old_roots
            .iter()
            .filter(|root| !new_roots.contains(root))
            .cloned()
            .collect(),
    }
}

/// The root sets to register (added) and retire (removed) around one commit.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct RootDiff {
    pub added: Vec<PathBuf>,
    pub removed: Vec<PathBuf>,
}

#[cfg(test)]
mod tests {
    use super::*;

    fn defaults() -> PolicyDefaults {
        PolicyDefaults {
            concurrency: 2,
            debounce: Duration::from_millis(1000),
            backend: WatchBackend::Native,
            gitignore: false,
            recovery_policy: crate::config::RecoveryPolicy::Prompt,
            recovery_timeout: Duration::from_secs(60),
            hooks: GenerationHooks::default(),
            session_hooks: crate::config::SessionHooks::default(),
        }
    }

    fn base_runtime(content: &str) -> RuntimeConfig {
        validate_candidate(content, std::env::current_dir().unwrap(), &defaults())
            .expect("valid base config")
    }

    #[test]
    fn invalid_yaml_fails_syntactic_gate() {
        let err = validate_candidate(
            "jobs: [unclosed",
            std::env::current_dir().unwrap(),
            &defaults(),
        )
        .expect_err("invalid yaml must fail");
        assert_eq!(err.gate, ValidationGate::Syntactic);
    }

    #[test]
    fn invalid_rule_value_fails_semantic_gate() {
        let err = validate_candidate(
            "jobs:\n  - name: x\n    run: echo hi\n    change: 'src/**'\n    service: not-a-bool\n",
            std::env::current_dir().unwrap(),
            &defaults(),
        )
        .expect_err("invalid rule value must fail");
        assert!(matches!(
            err.gate,
            ValidationGate::Syntactic | ValidationGate::Semantic
        ));
    }

    #[test]
    fn valid_candidate_builds_frozen_runtime() {
        let runtime =
            base_runtime("jobs:\n  - name: build\n    run: cargo build\n    change: 'src/**'\n");
        assert_eq!(runtime.rules.len(), 1);
        assert_eq!(runtime.concurrency, 2);
        assert_eq!(runtime.plan().task_names(), vec!["build"]);
    }

    #[test]
    fn candidate_declared_policy_wins_over_defaults() {
        // TASK-0092: the candidate's OWN concurrency/debounce/backend/hooks
        // participate in the frozen runtime — a policy change is semantic.
        let runtime = base_runtime(
            "on:\n  debounce: 250ms\n  watch_backend: poll\n  poll_interval: 100ms\nexecution:\n  concurrency: 8\nhooks:\n  success: 'echo done'\n  close: 'echo closed'\njobs:\n  - name: build\n    run: cargo build\n    change: 'src/**'\n",
        );
        assert_eq!(runtime.concurrency, 8);
        assert_eq!(runtime.debounce, Duration::from_millis(250));
        assert_eq!(
            runtime.backend,
            WatchBackend::Poll {
                interval: Duration::from_millis(100)
            }
        );
        assert_eq!(runtime.hooks.success.as_deref(), Some("echo done"));
        assert_eq!(runtime.session_hooks.close.as_deref(), Some("echo closed"));
    }

    #[test]
    fn candidate_recovery_timeout_is_frozen_and_reloadable() {
        let runtime = base_runtime(
            "execution:\n  recovery_timeout: 250ms\njobs:\n  - name: build\n    run: cargo build\n    change: 'src/**'\n",
        );
        assert_eq!(runtime.recovery_timeout, Duration::from_millis(250));
    }

    #[test]
    fn missing_policy_keys_keep_startup_defaults() {
        let runtime =
            base_runtime("jobs:\n  - name: build\n    run: cargo build\n    change: 'src/**'\n");
        assert_eq!(runtime.concurrency, defaults().concurrency);
        assert_eq!(runtime.debounce, defaults().debounce);
        assert_eq!(runtime.backend, defaults().backend);
        assert_eq!(runtime.hooks, GenerationHooks::default());
        assert_eq!(
            runtime.session_hooks,
            crate::config::SessionHooks::default()
        );
    }

    #[test]
    fn invalid_candidate_concurrency_is_semantic_fatal() {
        // `concurrency: 0` parses as YAML but fails the value gate; it must
        // be a Semantic fatal, never silently ignored.
        let err = validate_candidate(
            "execution:\n  concurrency: 0\njobs:\n  - name: build\n    run: cargo build\n    change: 'src/**'\n",
            std::env::current_dir().unwrap(),
            &defaults(),
        )
        .expect_err("zero concurrency must fail");
        assert_eq!(err.gate, ValidationGate::Semantic);
        assert!(err.reason.contains("concurrency"), "{}", err.reason);
    }

    #[test]
    fn candidate_control_socket_is_parsed_from_content() {
        // TASK-0092: the reload hash must see the candidate's `on.socket`
        // exactly like the startup capture — otherwise a config-declared
        // socket makes every save look like a semantic change.
        let with_socket = validate_candidate(
            "on:\n  socket: sock\njobs:\n  - name: build\n    run: cargo build\n    change: 'src/**'\n",
            std::env::current_dir().unwrap(),
            &defaults(),
        )
        .expect("valid with socket");
        assert_eq!(with_socket.control_socket, Some(PathBuf::from("sock")));

        let without_socket =
            base_runtime("jobs:\n  - name: build\n    run: cargo build\n    change: 'src/**'\n");
        assert_eq!(without_socket.control_socket, None);
        assert_ne!(
            crate::config_revision::semantic_hash(&with_socket),
            crate::config_revision::semantic_hash(&without_socket)
        );
    }

    #[test]
    fn decide_commits_on_semantic_change_and_noops_on_identical() {
        let mut tracker = RevisionTracker::new();
        let content = "jobs:\n  - name: build\n    run: cargo build\n    change: 'src/**'\n";
        let root = std::env::current_dir().unwrap();

        let first = decide(&mut tracker, content, root.clone(), &defaults());
        let ReloadDecision::Commit(r1) = first else {
            panic!("first observe must commit");
        };
        assert_eq!(r1.number, 1);

        // Same content (even reformatted) is a no-op.
        let noop = decide(
            &mut tracker,
            "# comment\njobs:\n  - name: build\n    change: 'src/**'\n    run: cargo build\n",
            root.clone(),
            &defaults(),
        );
        assert_eq!(noop, ReloadDecision::NoOp);

        // Semantic change commits revision 2.
        let second = decide(
            &mut tracker,
            "jobs:\n  - name: build\n    run: cargo test\n    change: 'src/**'\n",
            root,
            &defaults(),
        );
        let ReloadDecision::Commit(r2) = second else {
            panic!("command change must commit");
        };
        assert_eq!(r2.number, 2);
    }

    #[test]
    fn policy_change_in_candidate_is_semantic() {
        // TASK-0092 regression: a concurrency/debounce/hooks change in the
        // candidate must commit a new revision, never a no-op.
        let mut tracker = RevisionTracker::new();
        let root = std::env::current_dir().unwrap();
        let base = "jobs:\n  - name: build\n    run: cargo build\n    change: 'src/**'\n";

        let ReloadDecision::Commit(r1) = decide(&mut tracker, base, root.clone(), &defaults())
        else {
            panic!("first observe must commit");
        };
        assert_eq!(r1.number, 1);

        let changed = "on:\n  debounce: 250ms\nexecution:\n  concurrency: 8\nhooks:\n  success: 'echo done'\njobs:\n  - name: build\n    run: cargo build\n    change: 'src/**'\n";
        let ReloadDecision::Commit(r2) = decide(&mut tracker, changed, root.clone(), &defaults())
        else {
            panic!("policy change must commit");
        };
        assert_eq!(r2.number, 2);

        // Dropping the keys back to the defaults is also a change.
        let ReloadDecision::Commit(r3) = decide(&mut tracker, base, root, &defaults()) else {
            panic!("policy revert must commit");
        };
        assert_eq!(r3.number, 3);
    }

    #[test]
    fn decide_returns_fatal_for_invalid_candidate_without_mutating_tracker() {
        let mut tracker = RevisionTracker::new();
        let root = std::env::current_dir().unwrap();

        let fatal = decide(&mut tracker, "jobs: [unclosed", root, &defaults());
        let ReloadDecision::Fatal(error) = fatal else {
            panic!("invalid candidate must be fatal");
        };
        assert_eq!(error.gate, ValidationGate::Syntactic);
        assert!(
            tracker.current().is_none(),
            "invalid candidate must never publish a revision"
        );
    }

    #[test]
    fn root_diff_adds_removes_and_keeps_stable_order() {
        let old = vec![PathBuf::from("/repo/src"), PathBuf::from("/repo/tests")];
        let new = vec![PathBuf::from("/repo/src"), PathBuf::from("/repo/docs")];
        let diff = root_diff(&old, &new);
        assert_eq!(diff.added, vec![PathBuf::from("/repo/docs")]);
        assert_eq!(diff.removed, vec![PathBuf::from("/repo/tests")]);
    }

    #[test]
    fn root_diff_noop_when_sets_identical() {
        let roots = vec![PathBuf::from("/repo/src"), PathBuf::from("/repo/tests")];
        let diff = root_diff(&roots, &roots);
        assert!(diff.added.is_empty());
        assert!(diff.removed.is_empty());
    }

    #[test]
    fn root_diff_empty_old_adds_all_and_vice_versa() {
        let new = vec![PathBuf::from("/repo/src")];
        assert_eq!(
            root_diff(&[], &new),
            RootDiff {
                added: new.clone(),
                removed: vec![],
            }
        );
        assert_eq!(
            root_diff(&new, &[]),
            RootDiff {
                added: vec![],
                removed: new,
            }
        );
    }
}
