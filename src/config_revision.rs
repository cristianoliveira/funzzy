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

use crate::config::{GenerationHooks, SessionHooks};
use crate::plan::RunPlan;
use crate::rules::Rules;
use crate::watcher::WatchBackend;

/// Schema version of the canonical revision encoding. Bump only on a breaking
/// encoding change; bumping invalidates all old revision hashes.
pub const REVISION_SCHEMA_VERSION: u64 = 4;

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
    pub recovery_policy: crate::config::RecoveryPolicy,
    pub recovery_timeout: Duration,
    pub hooks: GenerationHooks,
    pub session_hooks: SessionHooks,
    /// Control socket path from `on.socket` (TASK-0090 AC8): part of the
    /// semantic surface so a socket path change is a real revision change
    /// and takes the bind-new-before-retire-old handoff, never a no-op.
    pub control_socket: Option<PathBuf>,
}

impl RuntimeConfig {
    /// Captures a complete immutable snapshot from the effective watch
    /// configuration. Callers (composition root) build it from a validated
    /// candidate; an invalid candidate never reaches this point.
    #[allow(clippy::too_many_arguments)]
    pub fn capture(
        root: PathBuf,
        rules: Vec<Rules>,
        concurrency: usize,
        debounce: Duration,
        backend: WatchBackend,
        respect_gitignore: bool,
        recovery_policy: crate::config::RecoveryPolicy,
        recovery_timeout: Duration,
        hooks: GenerationHooks,
        session_hooks: SessionHooks,
        control_socket: Option<PathBuf>,
    ) -> Self {
        Self {
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
        }
    }

    /// The execution plan for these frozen rules (unfiltered topology), so a
    /// generation's plan derives from the same revision as its policy.
    pub fn plan(&self) -> RunPlan {
        RunPlan::from_rules(self.rules.clone())
    }

    /// The managed services declared by this revision (TASK-0090 AC6): one
    /// `(name, service_signature)` per `service: true` rule, in config order.
    /// Signatures let the reload transaction keep unchanged services owned
    /// while changed/removed services are gracefully replaced/removed.
    pub fn services(&self) -> Vec<(String, String)> {
        self.rules
            .iter()
            .filter(|rule| rule.service())
            .map(|rule| (rule.name.clone(), service_signature(rule)))
            .collect()
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
    canonical.string(match config.recovery_policy {
        crate::config::RecoveryPolicy::Prompt => "prompt",
        crate::config::RecoveryPolicy::Skip => "skip",
    });
    canonical.u64(config.recovery_timeout.as_millis() as u64);
    canonical.string(&hooks_tag(&config.hooks));
    canonical.string(&format!("{:?}", config.session_hooks));
    // AC8: the control socket path is part of the semantic surface.
    canonical.optional_string(
        config
            .control_socket
            .as_ref()
            .map(|p| p.display().to_string()),
    );

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
    // MANUAL-TRIGGER-CONTRACT §7: trigger mode is semantic; absence is a
    // distinct canonical value from `manual` so a trigger-only reload never
    // hashes as a no-op.
    canonical.optional_string(rule.trigger().map(|mode| mode.as_str().to_owned()));
    // FINITE-JOB-TIMEOUT-CONTRACT §7: timeout is semantic (same pattern as
    // trigger; schema version 4) — a timeout-only reload never hashes as a
    // no-op; absence is distinct from a present budget.
    canonical.optional_u64(rule.timeout().map(|timeout| timeout.as_millis() as u64));
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

    match rule.recovery_commands() {
        Some(recovery) => {
            canonical.byte(1);
            canonical.u64(recovery.len() as u64);
            for command in recovery {
                canonical.string(&command);
            }
        }
        None => canonical.byte(0),
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

/// Per-service execution signature (TASK-0090 AC6): the canonical SHA-256
/// over the service rule's own semantic surface — name, service flag, output
/// policy, cwd, commands (hashed, never displayed), and environment KEYS
/// only. Two revisions with the same signature for a service name mean the
/// running process stays owned; a difference means graceful replacement.
/// Secrets-safe: environment VALUES never enter the digest.
pub fn service_signature(rule: &Rules) -> String {
    let mut canonical = CanonicalEncoder::new();
    canonical.u64(REVISION_SCHEMA_VERSION);
    canonical.bool(true); // service rule tag
    canonical.string(&rule.name);
    canonical.bool(rule.service());
    canonical.string(&output_policy_tag(&rule.output()));
    canonical.optional_string(rule.cwd().map(str::to_owned));

    let mut commands = rule.commands();
    commands.sort();
    canonical.u64(commands.len() as u64);
    for command in &commands {
        canonical.string(command);
    }

    let mut env_keys: Vec<&String> = rule.environment().keys().collect();
    env_keys.sort();
    canonical.u64(env_keys.len() as u64);
    for key in env_keys {
        canonical.string(key);
    }

    hex(&Sha256::digest(&canonical.bytes))
}

/// Stable output-policy tag for hashing.
fn output_policy_tag(policy: &crate::rules::OutputPolicy) -> String {
    match policy {
        crate::rules::OutputPolicy::Inherit => "inherit",
        crate::rules::OutputPolicy::Quiet => "quiet",
        crate::rules::OutputPolicy::Capture => "capture",
        crate::rules::OutputPolicy::ShowOnFailure => "show_on_failure",
    }
    .to_owned()
}

/// Stable hooks tag for hashing: which hooks are configured (command content
/// is hashed as part of the rule commands surface; here only presence and the
/// exact command strings matter semantically).
fn hooks_tag(hooks: &GenerationHooks) -> String {
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

    /// Optional u64 with presence discriminant (FINITE-JOB-TIMEOUT-
    /// CONTRACT §7: absence is a distinct canonical value from a budget).
    fn optional_u64(&mut self, value: Option<u64>) {
        match value {
            Some(value) => {
                self.byte(1);
                self.u64(value);
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
            crate::config::RecoveryPolicy::Prompt,
            Duration::from_secs(60),
            GenerationHooks::default(),
            SessionHooks::default(),
            None,
        )
    }

    #[test]
    fn semantic_hash_has_stable_sha256_fixture() {
        // Schema version 4 (MANUAL-TRIGGER-CONTRACT §7 + FINITE-JOB-TIMEOUT-
        // CONTRACT §7): trigger and timeout are encoded; fixture regenerated.
        let config = RuntimeConfig::capture(
            PathBuf::from("/workspace"),
            rules("jobs:\n  - name: build\n    run: cargo build\n    change: 'src/**'\n"),
            2,
            Duration::from_millis(1000),
            WatchBackend::Native,
            false,
            crate::config::RecoveryPolicy::Prompt,
            Duration::from_secs(60),
            GenerationHooks::default(),
            SessionHooks::default(),
            None,
        );
        assert_eq!(
            semantic_hash(&config),
            "af2582489f83464e35f5292f5bc87269df8c5d0486cd5fd85701328cd1660a7a"
        );
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

        let different_recovery_policy = RuntimeConfig {
            recovery_policy: crate::config::RecoveryPolicy::Skip,
            ..base.clone()
        };
        assert_ne!(
            semantic_hash(&base),
            semantic_hash(&different_recovery_policy)
        );
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

        let with_close_hook = RuntimeConfig {
            session_hooks: SessionHooks {
                close: Some("echo closed".to_owned()),
            },
            ..base.clone()
        };
        assert_ne!(semantic_hash(&base), semantic_hash(&with_close_hook));
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
    fn service_signature_is_stable_secret_safe_and_semantic() {
        // TASK-0090 AC6: per-service signature keeps unchanged services owned
        // and flags changed ones; environment VALUES never enter it.
        let svc = |config: &str| capture(rules(config)).services()[0].1.clone();
        let base = svc(
            "jobs:\n  - name: server\n    run: 'vite dev'\n    change: 'src/**'\n    service: true\n",
        );
        // Identical rule → identical signature.
        let same = svc(
            "jobs:\n  - name: server\n    run: 'vite dev'\n    change: 'src/**'\n    service: true\n",
        );
        assert_eq!(base, same);
        // Command change → signature change (must be replaced).
        let changed_command = svc(
            "jobs:\n  - name: server\n    run: 'vite dev --port 4000'\n    change: 'src/**'\n    service: true\n",
        );
        assert_ne!(base, changed_command);
        // Env VALUE change → same signature (secrets-safe); KEY change differs.
        let with_key = svc(
            "jobs:\n  - name: server\n    run: 'vite dev'\n    change: 'src/**'\n    service: true\n    env: { TOKEN: first }\n",
        );
        let value_changed = svc(
            "jobs:\n  - name: server\n    run: 'vite dev'\n    change: 'src/**'\n    service: true\n    env: { TOKEN: other-value }\n",
        );
        let key_added = svc(
            "jobs:\n  - name: server\n    run: 'vite dev'\n    change: 'src/**'\n    service: true\n    env: { TOKEN: first, EXTRA: k }\n",
        );
        assert_eq!(
            with_key, value_changed,
            "env values never enter the signature"
        );
        assert_ne!(with_key, key_added, "env keys are semantic");
    }

    #[test]
    fn control_socket_path_is_semantic_surface() {
        // TASK-0090 AC8: a socket path change is a real revision change so
        // the bind-new-before-retire-old handoff runs; identical socket is
        // a no-op.
        let base = capture(rules(
            "on:\n  socket: sock\njobs:\n  - name: build\n    run: cargo build\n    change: 'src/**'\n",
        ));
        let mut with_socket = base.clone();
        with_socket.control_socket = Some(std::path::PathBuf::from("sock"));
        let mut moved = with_socket.clone();
        moved.control_socket = Some(std::path::PathBuf::from("sock2"));
        let same = with_socket.clone();
        assert_ne!(
            semantic_hash(&with_socket),
            semantic_hash(&moved),
            "socket move is semantic"
        );
        assert_eq!(
            semantic_hash(&with_socket),
            semantic_hash(&same),
            "same socket is no-op"
        );
    }

    #[test]
    fn runtime_config_services_lists_only_service_rules_in_order() {
        let runtime = capture(rules(
            "jobs:\n  - name: build\n    run: cargo build\n    change: 'src/**'\n  - name: server\n    run: 'vite dev'\n    change: 'src/**'\n    service: true\n  - name: worker\n    run: 'sleep 1'\n    change: 'src/**'\n    service: true\n",
        ));
        let services = runtime.services();
        assert_eq!(services.len(), 2);
        assert_eq!(services[0].0, "server");
        assert_eq!(services[1].0, "worker");
        assert_ne!(services[0].1, services[1].1, "different services differ");
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
            crate::config::RecoveryPolicy::Prompt,
            Duration::from_secs(60),
            GenerationHooks::default(),
            SessionHooks::default(),
            None,
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

#[cfg(test)]
mod manual_trigger_tests {
    use super::*;
    use crate::config::GenerationHooks;
    use crate::rules::Rules;
    use std::time::Duration;

    fn manual_rule(trigger: bool) -> Rules {
        Rules::new(
            "build".to_owned(),
            vec!["cargo build".to_owned()],
            vec!["src/**".to_owned()],
            vec![],
            false,
        )
        .with_trigger(trigger.then_some(crate::rules::TriggerMode::Manual))
    }

    fn hash_for(rule: Rules) -> String {
        let config = RuntimeConfig::capture(
            PathBuf::from("/workspace"),
            vec![rule],
            2,
            Duration::from_millis(1000),
            WatchBackend::Native,
            false,
            crate::config::RecoveryPolicy::Prompt,
            Duration::from_secs(60),
            GenerationHooks::default(),
            SessionHooks::default(),
            None,
        );
        semantic_hash(&config)
    }

    #[test]
    fn trigger_mode_changes_the_semantic_hash() {
        // MANUAL-TRIGGER-CONTRACT §7: a trigger-only reload must not hash as
        // a no-op; absence is a distinct canonical value from `manual`.
        // This is also the TASK-0136 regression pin: the same rules,
        // commands, patterns, and policies — only trigger differs — must
        // produce different hashes (Kely's blocking-defect assertion).
        assert_ne!(hash_for(manual_rule(false)), hash_for(manual_rule(true)));
    }

    /// TASK-0136 regression pin: schema version 3 introduced trigger
    /// encoding; TASK-0139 bumps to 4 for timeout encoding (both invalidate
    /// prior revision hashes by design).
    #[test]
    fn revision_schema_version_is_pinned() {
        assert_eq!(REVISION_SCHEMA_VERSION, 4);
    }
    /// TASK-0136: a manual generation freezes its revision — a reload that
    /// edits ONLY the trigger mints a new revision (different hash), so the
    /// frozen generation can be attributed to the exact revision it ran
    /// under; formatting-only rewrites keep the same identity.
    #[test]
    fn trigger_only_edit_freezes_as_a_distinct_revision_for_running_generations() {
        let before = hash_for(manual_rule(true));
        let after = hash_for(manual_rule(false));
        assert_ne!(
            before, after,
            "a trigger-only reload is a semantic revision change, never a no-op"
        );
        // The frozen identity is deterministic: same config, same hash, so
        // attribution of an in-flight manual generation stays exact.
        assert_eq!(before, hash_for(manual_rule(true)));
    }

    #[test]
    fn formatting_only_rewrite_keeps_hash_and_trigger_is_deterministic() {
        assert_eq!(hash_for(manual_rule(true)), hash_for(manual_rule(true)));
    }
}

#[cfg(test)]
mod timeout_revision_tests {
    use super::*;
    use crate::config::GenerationHooks;
    use crate::rules::Rules;
    use std::time::Duration;

    fn rule_with(timeout: Option<Duration>) -> Rules {
        Rules::new(
            "build".to_owned(),
            vec!["cargo build".to_owned()],
            vec!["src/**".to_owned()],
            vec![],
            false,
        )
        .with_timeout(timeout)
    }

    fn hash_for(rule: Rules) -> String {
        let config = RuntimeConfig::capture(
            PathBuf::from("/workspace"),
            vec![rule],
            2,
            Duration::from_millis(1000),
            WatchBackend::Native,
            false,
            crate::config::RecoveryPolicy::Prompt,
            Duration::from_secs(60),
            GenerationHooks::default(),
            SessionHooks::default(),
            None,
        );
        semantic_hash(&config)
    }

    /// FINITE-JOB-TIMEOUT-CONTRACT §7: a timeout-only reload never hashes as
    /// a no-op; absence, 200ms, and 30m are three distinct identities.
    #[test]
    fn timeout_is_semantic_and_absence_is_distinct() {
        let none = hash_for(rule_with(None));
        let small = hash_for(rule_with(Some(Duration::from_millis(200))));
        let large = hash_for(rule_with(Some(Duration::from_secs(1800))));
        assert_ne!(none, small);
        assert_ne!(none, large);
        assert_ne!(small, large);
    }
}
