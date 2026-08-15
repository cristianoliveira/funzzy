//! Config lifecycle state source (TASK-0091, CONFIG-RELOAD-CONTRACT §3/§5).
//!
//! One shared source of truth for configuration lifecycle transitions:
//! `configReloading` (a valid candidate is being prepared/committed),
//! `configReloaded` (the commit boundary passed), and terminal
//! `configInvalid` (the watcher is shutting down fatally). Formatting-only
//! no-op saves never transition this source — the stdout notice is the only
//! explicit signal, and subsystems (revision, snapshots, subscriptions) stay
//! quiet per contract §4.
//!
//! The source is bounded: a fixed-size transition history, newest last, so a
//! control client can reconstruct the recent config story without unbounded
//! memory. Watchers (the snapshot broker) register callbacks and are invoked
//! on every transition so subscriptions observe reloads without polling.

use crate::config_revision::ConfigRevision;
use serde_derive::Serialize;
use std::collections::VecDeque;
use std::sync::{Arc, Mutex};

/// Bounded transition history retained by the state source.
pub const LIFECYCLE_HISTORY_BOUND: usize = 32;

/// Config lifecycle phase (wire values are exactly the contract event names).
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub enum ConfigPhase {
    /// No config transition yet; the watcher runs its initial revision.
    Idle,
    /// A valid candidate passed validation and is being committed.
    ConfigReloading,
    /// The commit boundary passed; a new revision is live.
    ConfigReloaded,
    /// Terminal: an invalid candidate; the watcher shuts down fatally.
    ConfigInvalid,
}

/// One observable lifecycle transition (additive control payload).
#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ConfigTransition {
    /// The phase after this transition.
    pub phase: ConfigPhase,
    /// Target revision (Reloading) or committed revision (Reloaded); the
    /// live revision at the time of an Invalid transition.
    pub revision: Option<u64>,
    /// Non-secret semantic hash of the revision above.
    pub revision_hash: Option<String>,
    /// Human-readable reason: the failed gate + reason for Invalid; None
    /// for valid transitions.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub reason: Option<String>,
    /// Monotonic transition ordinal; never reused within one process.
    pub ordinal: u64,
    /// Wall-clock transition time (epoch ms).
    pub at_epoch_ms: u64,
}

impl ConfigTransition {
    fn new(
        phase: ConfigPhase,
        revision: Option<u64>,
        revision_hash: Option<String>,
        reason: Option<String>,
        ordinal: u64,
    ) -> Self {
        Self {
            phase,
            revision,
            revision_hash,
            reason,
            ordinal,
            at_epoch_ms: now_epoch_ms(),
        }
    }
}

/// Observer callback invoked after a transition is recorded. The callback
/// runs OUTSIDE the lifecycle lock, so observers may safely lock other
/// subsystems without a lock-order inversion.
type LifecycleWatcher = Arc<dyn Fn(&ConfigTransition) + Send + Sync>;
struct LifecycleInner {
    current: ConfigTransition,
    history: VecDeque<ConfigTransition>,
    ordinal: u64,
    watchers: Vec<LifecycleWatcher>,
}

/// Shared config lifecycle source. The reload thread writes transitions; the
/// snapshot broker and control server read them.
pub struct ConfigLifecycle {
    inner: Mutex<LifecycleInner>,
}

impl Default for ConfigLifecycle {
    fn default() -> Self {
        Self::new()
    }
}

impl ConfigLifecycle {
    pub fn new() -> Self {
        Self {
            inner: Mutex::new(LifecycleInner {
                current: ConfigTransition::new(ConfigPhase::Idle, None, None, None, 0),
                history: VecDeque::new(),
                ordinal: 0,
                watchers: vec![],
            }),
        }
    }

    /// Records `configReloading` for a candidate about to commit. The target
    /// revision is the candidate's (already validated) revision.
    pub fn reloading(&self, revision: Option<&ConfigRevision>) -> ConfigTransition {
        self.transition(ConfigPhase::ConfigReloading, revision, None)
    }

    /// Records `configReloaded` after the commit boundary passed.
    pub fn reloaded(&self, revision: &ConfigRevision) -> ConfigTransition {
        self.transition(ConfigPhase::ConfigReloaded, Some(revision), None)
    }

    /// Records terminal `configInvalid` right before fatal shutdown. The
    /// revision facts are the live revision at the time of the failure.
    pub fn invalid(
        &self,
        live_revision: Option<&ConfigRevision>,
        reason: String,
    ) -> ConfigTransition {
        self.transition(ConfigPhase::ConfigInvalid, live_revision, Some(reason))
    }

    /// The current lifecycle transition (never empty; starts at `Idle`).
    pub fn current(&self) -> ConfigTransition {
        self.inner.lock().unwrap().current.clone()
    }

    /// The bounded transition history, oldest first. `Idle` is not part of
    /// history; only transitions after it are recorded.
    pub fn history(&self) -> Vec<ConfigTransition> {
        self.inner.lock().unwrap().history.iter().cloned().collect()
    }

    /// Registers a watcher invoked on every transition after it is recorded
    /// (outside the lock), so the snapshot broker can publish subscriptions
    /// on reload transitions without polling. Process-lifetime registration:
    /// the broker lives as long as the watcher, so watchers are never
    /// removed.
    pub fn watch(&self, watcher: LifecycleWatcher) {
        self.inner.lock().unwrap().watchers.push(watcher);
    }

    fn transition(
        &self,
        phase: ConfigPhase,
        revision: Option<&ConfigRevision>,
        reason: Option<String>,
    ) -> ConfigTransition {
        let mut inner = self.inner.lock().unwrap();
        inner.ordinal += 1;
        let transition = ConfigTransition::new(
            phase,
            revision.map(|r| r.number),
            revision.map(|r| r.hash.clone()),
            reason,
            inner.ordinal,
        );
        inner.current = transition.clone();
        inner.history.push_back(transition.clone());
        while inner.history.len() > LIFECYCLE_HISTORY_BOUND {
            inner.history.pop_front();
        }
        let watchers: Vec<LifecycleWatcher> = inner.watchers.clone();
        drop(inner);
        for watcher in &watchers {
            watcher(&transition);
        }
        transition
    }
}

fn now_epoch_ms() -> u64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|duration| duration.as_millis() as u64)
        .unwrap_or(0)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn revision(number: u64) -> ConfigRevision {
        ConfigRevision {
            number,
            hash: format!("hash-{number}"),
        }
    }

    #[test]
    fn starts_idle_with_no_history() {
        let lifecycle = ConfigLifecycle::new();
        let current = lifecycle.current();
        assert_eq!(current.phase, ConfigPhase::Idle);
        assert_eq!(current.ordinal, 0);
        assert!(current.revision.is_none());
        assert!(lifecycle.history().is_empty());
    }

    #[test]
    fn reloading_then_reloaded_records_both_in_order() {
        let lifecycle = ConfigLifecycle::new();
        let loading = lifecycle.reloading(Some(&revision(2)));
        assert_eq!(loading.phase, ConfigPhase::ConfigReloading);
        assert_eq!(loading.revision, Some(2));
        assert_eq!(loading.ordinal, 1);
        assert!(loading.reason.is_none());

        let loaded = lifecycle.reloaded(&revision(2));
        assert_eq!(loaded.phase, ConfigPhase::ConfigReloaded);
        assert_eq!(loaded.revision, Some(2));
        assert_eq!(loaded.revision_hash.as_deref(), Some("hash-2"));
        assert_eq!(loaded.ordinal, 2);

        let history = lifecycle.history();
        assert_eq!(history.len(), 2);
        assert_eq!(history[0], loading);
        assert_eq!(history[1], loaded);
        assert_eq!(lifecycle.current(), loaded);
    }

    #[test]
    fn invalid_is_terminal_and_carries_reason_with_live_revision() {
        let lifecycle = ConfigLifecycle::new();
        let invalid = lifecycle.invalid(Some(&revision(1)), "semantics: bad glob".to_owned());
        assert_eq!(invalid.phase, ConfigPhase::ConfigInvalid);
        assert_eq!(invalid.revision, Some(1));
        assert_eq!(invalid.reason.as_deref(), Some("semantics: bad glob"));
        assert_eq!(lifecycle.current(), invalid);
    }

    #[test]
    fn history_is_bounded_newest_last() {
        let lifecycle = ConfigLifecycle::new();
        for number in 1..=(LIFECYCLE_HISTORY_BOUND + 5) as u64 {
            lifecycle.reloaded(&revision(number));
        }
        let history = lifecycle.history();
        assert_eq!(history.len(), LIFECYCLE_HISTORY_BOUND);
        assert_eq!(
            history.first().unwrap().revision,
            Some(6),
            "oldest transitions are evicted"
        );
        assert_eq!(
            history.last().unwrap().revision,
            Some(LIFECYCLE_HISTORY_BOUND as u64 + 5),
            "newest is the last transition"
        );
        assert_eq!(
            lifecycle.current().revision,
            Some(LIFECYCLE_HISTORY_BOUND as u64 + 5)
        );
    }

    #[test]
    fn watchers_are_invoked_outside_the_lock_after_each_transition() {
        let lifecycle = Arc::new(ConfigLifecycle::new());
        let seen: Arc<Mutex<Vec<ConfigPhase>>> = Arc::new(Mutex::new(vec![]));
        let seen_cb = Arc::clone(&seen);
        lifecycle.watch(Arc::new(move |transition| {
            seen_cb.lock().unwrap().push(transition.phase);
        }));

        lifecycle.reloading(Some(&revision(2)));
        lifecycle.reloaded(&revision(2));
        assert_eq!(
            *seen.lock().unwrap(),
            vec![ConfigPhase::ConfigReloading, ConfigPhase::ConfigReloaded]
        );
    }

    #[test]
    fn transitions_serialize_with_the_contract_event_names() {
        let transition = ConfigTransition::new(
            ConfigPhase::ConfigInvalid,
            Some(1),
            Some("hash-1".to_owned()),
            Some("broken".to_owned()),
            3,
        );
        let json = serde_json::to_value(&transition).unwrap();
        assert_eq!(json["phase"], "configInvalid");
        assert_eq!(json["revision"], 1);
        assert_eq!(json["revisionHash"], "hash-1");
        assert_eq!(json["reason"], "broken");
        assert_eq!(json["ordinal"], 3);

        let loading = ConfigTransition::new(ConfigPhase::ConfigReloading, None, None, None, 1);
        let json = serde_json::to_value(&loading).unwrap();
        assert_eq!(json["phase"], "configReloading");
        assert!(
            json.get("reason").is_none(),
            "no reason key for valid phases"
        );
    }
}
