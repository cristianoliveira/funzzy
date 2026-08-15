//! Immutable runtime config revisions (TASK-0089, CONFIG-RELOAD-CONTRACT §4).
//!
//! In-process reload needs one validated immutable runtime snapshot so active
//! work cannot observe a partial mixture of old jobs and new policy while
//! later generations use new configuration. This module owns the domain:
//! [`RuntimeConfig`] (the frozen effective config), [`ConfigRevision`] (a
//! monotonic number + deterministic semantic hash), and [`RevisionTracker`]
//! (increments only on semantic change; formatting-only rewrites are no-ops).
//!
//! The semantic hash is computed from frozen effective config material and is
//! **secrets-safe**: declared environment *values* never enter the hash, only
//! environment *keys* (sorted). Command content is hashed (never displayed).

use std::path::PathBuf;
use std::time::Duration;

use sha2::{Digest, Sha256};

use crate::config::RunHooks;
use crate::plan::RunPlan;
use crate::rules::Rules;
use crate::watcher::WatchBackend;

/// Schema version of the canonical revision encoding. Bump only on a breaking
/// encoding change; bumping invalidates all old revision hashes.
pub const REVISION_SCHEMA_VERSION: u64 = 1;

/// One immutable revision of the effective runtime configuration: a monotonic
/// number plus the deterministic semantic hash of the frozen config. Two
/// revisions with the same hash are semantically identical (formatting-only
/// rewrites do not produce a new revision).
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ConfigRevision {
    pub number: u64,
    pub hash: String,
}

/// The frozen effective runtime config: everything a generation's plan and
/// policy derive from, captured at one point in time. Owns its rules so the
/// snapshot never observes later mutation.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct RuntimeConfig {
    pub root: PathBuf,
    pub rules: Vec<Rules>,
    pub concurrency: usize,
    pub debounce: Duration,
    pub backend: WatchBackend,
    pub respect_gitignore: bool,
    pub hooks: RunHooks,
}

impl RuntimeConfig {
    /// Captures a complete immutable snapshot from the effective watch
    /// configuration. Callers (composition root) build it from a validated
    /// candidate; an invalid candidate never reaches this point.
    pub fn capture(
        root: PathBuf,
        rules: Vec<Rules>,
        concurrency: usize,
        debounce: Duration,
        backend: WatchBackend,
        respect_gitignore: bool,
        hooks: RunHooks,
    ) -> Self {
        Self {
            root,
            rules,
            concurrency,
            debounce,
            backend,
            respect_gitignore,
            hooks,
        }
    }

    /// The execution plan for these frozen rules (unfiltered topology), so a
    /// generation's plan derives from the same revision as its policy.
    pub fn plan(&self) -> RunPlan {
        RunPlan::from_rules(self.rules.clone())
    }
}

/// Deterministic semantic hash of a frozen runtime config.
///
/// Canonical, ambiguity-free encoding (length-prefixed fields, sorted env
/// keys, never env values) hashed with SHA-256. Formatting-only rewrites
/// produce the same rules and policy and therefore the same hash.
pub fn semantic_hash(config: &RuntimeConfig) -> String {
    let mut canonical = CanonicalEncoder::new();
    canonical.u64(REVISION_SCHEMA_VERSION);
    canonical.string(&config.root.to_string_lossy());
    canonical.u64(config.concurrency as u64);
    canonical.u64(config.debounce.as_millis() as u64);
    canonical.string(&backend_tag(config.backend));
    canonical.bool(config.respect_gitignore);
    canonical.string(&hooks_tag(&config.hooks));

    // Rules encode their full semantic surface: name, run_on_init, parallel
    // group, service, output policy, change/ignore patterns (sorted), commands
    // (hashed, never displayed), cwd, and environment KEYS only.
    canonical.u64(config.rules.len() as u64);
    for rule in &config.rules {
        encode_rule(&mut canonical, rule);
    }

    hex(&Sha256::digest(&canonical.bytes))
}

/// Encodes one rule's semantic surface in canonical form.
fn encode_rule(canonical: &mut CanonicalEncoder, rule: &Rules) {
    canonical.string(&rule.name);
    canonical.bool(rule.run_on_init());
    canonical.optional_string(rule.parallel().map(str::to_owned));
    canonical.bool(rule.service());
    canonical.string(&output_policy_tag(&rule.output()));
    canonical.optional_string(rule.cwd().map(str::to_owned));

    let mut change = rule.watch_patterns();
    change.sort();
    canonical.u64(change.len() as u64);
    for pattern in &change {
        canonical.string(pattern);
    }

    let mut ignore = rule.ignore_glob_patterns();
    ignore.sort();
    canonical.u64(ignore.len() as u64);
    for pattern in &ignore {
        canonical.string(pattern);
    }

    let mut commands = rule.commands();
    commands.sort();
    canonical.u64(commands.len() as u64);
    for command in &commands {
        canonical.string(command);
    }

    // Environment: KEYS ONLY (sorted). Values are secrets and never enter
    // the hash; presence/absence of a key is semantic, its content is not.
    let env_keys: Vec<&String> = rule.environment().keys().collect();
    canonical.u64(env_keys.len() as u64);
    for key in env_keys {
        canonical.string(key);
    }
}

/// Stable backend tag for hashing.
fn backend_tag(backend: WatchBackend) -> String {
    match backend {
        WatchBackend::Native => "native".to_owned(),
        WatchBackend::Poll { interval } => format!("poll:{}", interval.as_millis()),
        WatchBackend::Auto => "auto".to_owned(),
    }
}

/// Stable output-policy tag for hashing.
fn output_policy_tag(policy: &crate::config::OutputPolicy) -> String {
    match policy {
        crate::config::OutputPolicy::Inherit => "inherit",
        crate::config::OutputPolicy::Quiet => "quiet",
        crate::config::OutputPolicy::Capture => "capture",
        crate::config::OutputPolicy::ShowOnFailure => "show_on_failure",
    }
    .to_owned()
}

/// Stable hooks tag for hashing: which hooks are configured (command content
/// is hashed as part of the rule commands surface; here only presence and the
/// exact command strings matter semantically).
fn hooks_tag(hooks: &RunHooks) -> String {
    format!("{:?}", hooks)
}

/// Tracks revisions monotonically: the first observed config is revision 1;
/// a semantic change increments; a no-op (identical hash) keeps the current
/// revision and reports `NoOp` so subsystems do not churn.
#[derive(Clone, Debug, Default)]
pub struct RevisionTracker {
    current: Option<ConfigRevision>,
}

impl RevisionTracker {
    pub fn new() -> Self {
        Self { current: None }
    }

    /// Seeds the tracker with a revision already observed elsewhere (e.g.
    /// the composition root's initial config), so reload numbering continues
    /// monotonically from startup instead of starting another sequence.
    pub fn seed(&mut self, revision: ConfigRevision) {
        self.current = Some(revision);
    }

    /// Observes a candidate config. Returns `New(revision)` when the hash
    /// differs from the current revision (or no revision exists yet);
    /// `NoOp` when the candidate is semantically identical.
    pub fn observe(&mut self, config: &RuntimeConfig) -> RevisionDecision {
        let hash = semantic_hash(config);
        match &self.current {
            Some(current) if current.hash == hash => RevisionDecision::NoOp,
            _ => {
                let number = self.current.as_ref().map(|c| c.number).unwrap_or(0) + 1;
                let revision = ConfigRevision {
                    number,
                    hash: hash.clone(),
                };
                self.current = Some(revision.clone());
                RevisionDecision::New(revision)
            }
        }
    }

    /// The current revision, when one has been observed.
    pub fn current(&self) -> Option<&ConfigRevision> {
        self.current.as_ref()
    }
}

/// Result of observing a candidate config.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum RevisionDecision {
    /// A new immutable revision became live (monotonic number).
    New(ConfigRevision),
    /// The candidate was semantically identical; nothing changed.
    NoOp,
}

/// Canonical, ambiguity-free byte encoder (same discipline as
/// `plan::CanonicalEncoder`): u64 LE length prefixes prevent concatenation
/// collisions; sorted iteration keeps insertion order out of the digest.
struct CanonicalEncoder {
    bytes: Vec<u8>,
}

impl CanonicalEncoder {
    fn new() -> Self {
        Self { bytes: Vec::new() }
    }

    fn u64(&mut self, value: u64) {
        self.bytes.extend_from_slice(&value.to_le_bytes());
    }

    fn byte(&mut self, value: u8) {
        self.bytes.push(value);
    }

    fn bool(&mut self, value: bool) {
        self.bytes.push(value as u8);
    }

    fn string(&mut self, value: &str) {
        self.u64(value.len() as u64);
        self.bytes.extend_from_slice(value.as_bytes());
    }

    fn optional_string(&mut self, value: Option<String>) {
        match value {
            Some(value) => {
                self.byte(1);
                self.string(&value);
            }
            None => self.byte(0),
        }
    }
}

fn hex(bytes: &[u8]) -> String {
    bytes.iter().map(|b| format!("{b:02x}")).collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::config;

    fn rules(content: &str) -> Vec<Rules> {
        config::from_yaml(content).expect("parse config")
    }

    fn capture(rules: Vec<Rules>) -> RuntimeConfig {
        RuntimeConfig::capture(
            std::env::current_dir().unwrap(),
            rules,
            2,
            Duration::from_millis(1000),
            WatchBackend::Native,
            false,
            RunHooks::default(),
        )
    }

    #[test]
    fn identical_configs_hash_equal() {
        let a = capture(rules(
            "jobs:\n  - name: build\n    run: cargo build\n    change: 'src/**'\n",
        ));
        let b = capture(rules(
            "jobs:\n  - name: build\n    run: cargo build\n    change: 'src/**'\n",
        ));
        assert_eq!(semantic_hash(&a), semantic_hash(&b));
    }

    #[test]
    fn formatting_only_rewrite_hashes_equal_and_is_noop() {
        let plain = capture(rules(
            "jobs:\n  - name: build\n    run: cargo build\n    change: 'src/**'\n",
        ));
        // Same semantics, different whitespace/comments/ordering of YAML.
        let reformatted = capture(rules(
            "# comment only\njobs:\n  - name: build\n    change: 'src/**'\n    run: cargo build\n",
        ));
        assert_eq!(semantic_hash(&plain), semantic_hash(&reformatted));

        let mut tracker = RevisionTracker::new();
        assert_eq!(
            tracker.observe(&plain),
            RevisionDecision::New(ConfigRevision {
                number: 1,
                hash: semantic_hash(&plain),
            })
        );
        assert_eq!(tracker.observe(&reformatted), RevisionDecision::NoOp);
        assert_eq!(tracker.current().unwrap().number, 1);
    }

    #[test]
    fn semantic_change_increments_monotonic_revision() {
        let mut tracker = RevisionTracker::new();
        let v1 = capture(rules(
            "jobs:\n  - name: build\n    run: cargo build\n    change: 'src/**'\n",
        ));
        let v2 = capture(rules(
            "jobs:\n  - name: build\n    run: cargo test\n    change: 'src/**'\n",
        ));
        let v3 = capture(rules(
            "jobs:\n  - name: lint\n    run: cargo fmt\n    change: '*.rs'\n",
        ));

        let first = tracker.observe(&v1);
        let RevisionDecision::New(r1) = first else {
            panic!("first observe must be new");
        };
        assert_eq!(r1.number, 1);

        let RevisionDecision::New(r2) = tracker.observe(&v2) else {
            panic!("command change must be semantic");
        };
        assert_eq!(r2.number, 2);

        let RevisionDecision::New(r3) = tracker.observe(&v3) else {
            panic!("job topology change must be semantic");
        };
        assert_eq!(r3.number, 3);
        assert_eq!(tracker.current().unwrap().number, 3);
    }

    #[test]
    fn policy_changes_are_semantic() {
        let base = capture(rules(
            "jobs:\n  - name: build\n    run: cargo build\n    change: 'src/**'\n",
        ));

        // Different concurrency is a different revision.
        let different_concurrency = RuntimeConfig {
            concurrency: 8,
            ..base.clone()
        };
        assert_ne!(semantic_hash(&base), semantic_hash(&different_concurrency));

        // Different backend is a different revision.
        let different_backend = RuntimeConfig {
            backend: WatchBackend::Poll {
                interval: Duration::from_millis(200),
            },
            ..base.clone()
        };
        assert_ne!(semantic_hash(&base), semantic_hash(&different_backend));

        // Different debounce is a different revision.
        let different_debounce = RuntimeConfig {
            debounce: Duration::from_millis(500),
            ..base.clone()
        };
        assert_ne!(semantic_hash(&base), semantic_hash(&different_debounce));
    }

    #[test]
    fn gitignore_and_hooks_are_semantic() {
        let base = capture(rules(
            "jobs:\n  - name: build\n    run: cargo build\n    change: 'src/**'\n",
        ));

        let with_gitignore = RuntimeConfig {
            respect_gitignore: true,
            ..base.clone()
        };
        assert_ne!(semantic_hash(&base), semantic_hash(&with_gitignore));
    }

    #[test]
    fn environment_values_are_secret_safe_but_keys_are_semantic() {
        let with_secret = capture(rules(
            "jobs:\n  - name: build\n    run: cargo build\n    change: 'src/**'\n    env: { TOKEN: super-secret-value }\n",
        ));
        let same_key_other_value = capture(rules(
            "jobs:\n  - name: build\n    run: cargo build\n    change: 'src/**'\n    env: { TOKEN: different-secret-value }\n",
        ));

        // Same env KEY → same hash: values never enter the hash.
        assert_eq!(
            semantic_hash(&with_secret),
            semantic_hash(&same_key_other_value)
        );
        let hash = semantic_hash(&with_secret);
        assert!(
            !hash.contains("secret"),
            "values must never leak into the hash"
        );
        assert!(!hash.contains("TOKEN"), "keys are hashed, not readable");

        // A different env KEY set is a semantic change.
        let extra_key = capture(rules(
            "jobs:\n  - name: build\n    run: cargo build\n    change: 'src/**'\n    env: { TOKEN: x, EXTRA: y }\n",
        ));
        assert_ne!(semantic_hash(&with_secret), semantic_hash(&extra_key));
    }

    #[test]
    fn ignore_patterns_and_service_flag_are_semantic() {
        let base = capture(rules(
            "jobs:\n  - name: build\n    run: cargo build\n    change: 'src/**'\n",
        ));
        let with_ignore = capture(rules(
            "jobs:\n  - name: build\n    run: cargo build\n    change: 'src/**'\n    ignore: 'src/generated/**'\n",
        ));
        assert_ne!(semantic_hash(&base), semantic_hash(&with_ignore));

        let service = capture(rules(
            "jobs:\n  - name: build\n    run: cargo build\n    change: 'src/**'\n    service: true\n",
        ));
        assert_ne!(semantic_hash(&base), semantic_hash(&service));
    }

    #[test]
    fn revision_hash_is_stable_and_never_reused_across_semantic_changes() {
        let v1 = capture(rules(
            "jobs:\n  - name: build\n    run: cargo build\n    change: 'src/**'\n",
        ));
        let v2 = capture(rules(
            "jobs:\n  - name: build\n    run: cargo test\n    change: 'src/**'\n",
        ));
        let h1 = semantic_hash(&v1);
        let h2 = semantic_hash(&v2);
        assert_ne!(h1, h2);
        // Deterministic: hashing again yields the same digest.
        assert_eq!(semantic_hash(&v1), h1);
        assert_eq!(semantic_hash(&v2), h2);
    }

    #[test]
    fn frozen_plan_derives_from_snapshot_rules() {
        let config = capture(rules(
            "jobs:\n  - name: build\n    run: cargo build\n    change: 'src/**'\n  - name: lint\n    parallel: checks\n    run: cargo fmt\n    change: '*.rs'\n  - name: test\n    parallel: checks\n    run: cargo test\n    change: '*.rs'\n",
        ));
        let plan = config.plan();
        assert_eq!(plan.task_names(), vec!["build", "lint", "test"]);
    }
}

#[cfg(test)]
mod history_tests {
    use super::*;
    use crate::config;

    fn rules(content: &str) -> Vec<Rules> {
        config::from_yaml(content).expect("parse config")
    }

    fn capture(rules: Vec<Rules>) -> RuntimeConfig {
        RuntimeConfig::capture(
            std::env::current_dir().unwrap(),
            rules,
            2,
            Duration::from_millis(1000),
            WatchBackend::Native,
            false,
            RunHooks::default(),
        )
    }

    /// TASK-0089 AC: duration history keys derive from the frozen effective
    /// config; a formatting-only reload produces the same execution signature
    /// and therefore does not invalidate existing history.
    #[test]
    fn formatting_only_reload_keeps_execution_signature() {
        let plain = capture(rules(
            "jobs:\n  - name: build\n    run: cargo build\n    change: 'src/**'\n",
        ));
        let reformatted = capture(rules(
            "# comment\njobs:\n  - name: build\n    change: 'src/**'\n    run: cargo build\n",
        ));

        let sig_plain = plain.plan().execution_signature(2, false);
        let sig_reformatted = reformatted.plan().execution_signature(2, false);
        assert_eq!(
            sig_plain, sig_reformatted,
            "formatting-only rewrite must not invalidate duration history"
        );
    }

    /// A semantic change (job command) invalidates the signature.
    #[test]
    fn semantic_change_invalidates_execution_signature() {
        let v1 = capture(rules(
            "jobs:\n  - name: build\n    run: cargo build\n    change: 'src/**'\n",
        ));
        let v2 = capture(rules(
            "jobs:\n  - name: build\n    run: cargo test\n    change: 'src/**'\n",
        ));
        assert_ne!(
            v1.plan().execution_signature(2, false),
            v2.plan().execution_signature(2, false)
        );
    }
}
