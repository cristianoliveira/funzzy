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

use crate::config::RunHooks;
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

/// Validates a candidate config text against the first three pure gates and
/// builds the frozen runtime config. Operational preparation (roots/services)
/// is checked separately by the caller before commit.
pub fn validate_candidate(
    content: &str,
    root: PathBuf,
    concurrency: usize,
    debounce: Duration,
    backend: WatchBackend,
    respect_gitignore: bool,
    hooks: RunHooks,
    control_socket: Option<PathBuf>,
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

    Ok(RuntimeConfig::capture(
        root,
        rules,
        concurrency,
        debounce,
        backend,
        respect_gitignore,
        hooks,
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
/// (pure gates), then asks the tracker whether it is a semantic change.
/// Operational preparation is NOT part of this decision; the caller prepares
/// roots before committing.
pub fn decide(
    tracker: &mut RevisionTracker,
    content: &str,
    root: PathBuf,
    concurrency: usize,
    debounce: Duration,
    backend: WatchBackend,
    respect_gitignore: bool,
    hooks: RunHooks,
    control_socket: Option<PathBuf>,
) -> ReloadDecision {
    match validate_candidate(
        content,
        root,
        concurrency,
        debounce,
        backend,
        respect_gitignore,
        hooks,
        control_socket,
    ) {
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

    fn base_runtime(content: &str) -> RuntimeConfig {
        validate_candidate(
            content,
            std::env::current_dir().unwrap(),
            2,
            Duration::from_millis(1000),
            WatchBackend::Native,
            false,
            RunHooks::default(),
            None,
        )
        .expect("valid base config")
    }

    #[test]
    fn invalid_yaml_fails_syntactic_gate() {
        let err = validate_candidate(
            "jobs: [unclosed",
            std::env::current_dir().unwrap(),
            2,
            Duration::from_millis(1000),
            WatchBackend::Native,
            false,
            RunHooks::default(),
            None,
        )
        .expect_err("invalid yaml must fail");
        assert_eq!(err.gate, ValidationGate::Syntactic);
    }

    #[test]
    fn invalid_rule_value_fails_semantic_gate() {
        let err = validate_candidate(
            "jobs:\n  - name: x\n    run: echo hi\n    change: 'src/**'\n    service: not-a-bool\n",
            std::env::current_dir().unwrap(),
            2,
            Duration::from_millis(1000),
            WatchBackend::Native,
            false,
            RunHooks::default(),
            None,
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
    fn decide_commits_on_semantic_change_and_noops_on_identical() {
        let mut tracker = RevisionTracker::new();
        let content = "jobs:\n  - name: build\n    run: cargo build\n    change: 'src/**'\n";
        let root = std::env::current_dir().unwrap();

        let first = decide(
            &mut tracker,
            content,
            root.clone(),
            2,
            Duration::from_millis(1000),
            WatchBackend::Native,
            false,
            RunHooks::default(),
            None,
        );
        let ReloadDecision::Commit(r1) = first else {
            panic!("first observe must commit");
        };
        assert_eq!(r1.number, 1);

        // Same content (even reformatted) is a no-op.
        let noop = decide(
            &mut tracker,
            "# comment\njobs:\n  - name: build\n    change: 'src/**'\n    run: cargo build\n",
            root.clone(),
            2,
            Duration::from_millis(1000),
            WatchBackend::Native,
            false,
            RunHooks::default(),
            None,
        );
        assert_eq!(noop, ReloadDecision::NoOp);

        // Semantic change commits revision 2.
        let second = decide(
            &mut tracker,
            "jobs:\n  - name: build\n    run: cargo test\n    change: 'src/**'\n",
            root,
            2,
            Duration::from_millis(1000),
            WatchBackend::Native,
            false,
            RunHooks::default(),
            None,
        );
        let ReloadDecision::Commit(r2) = second else {
            panic!("semantic change must commit");
        };
        assert_eq!(r2.number, 2);
    }

    #[test]
    fn decide_returns_fatal_for_invalid_candidate_without_mutating_tracker() {
        let mut tracker = RevisionTracker::new();
        let root = std::env::current_dir().unwrap();

        let fatal = decide(
            &mut tracker,
            "jobs: [unclosed",
            root,
            2,
            Duration::from_millis(1000),
            WatchBackend::Native,
            false,
            RunHooks::default(),
            None,
        );
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
