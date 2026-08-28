//! Correlated snapshot and subscription broker (TASK-0050, contract §7).
//!
//! The snapshot reuses the same injected event source as atomic await: it
//! reads `WatcherState` (latest generation) plus the `AwaitCoordinator`
//! (pending-work and freshness facts) under the established lock order. The
//! broker owns subscribers and the bounded per-subscriber notification
//! channel; it never builds a second state tracker.

use crate::awaiting::{classify, AwaitCoordinator};
use crate::config_lifecycle::{ConfigLifecycle, ConfigTransition};
use crate::duration_history::RunEstimate;
use crate::executor::TaskSnapshot;
use crate::output::DEFAULT_FAILURE_EVIDENCE_LINES;
use crate::watcher_state::{WatcherExecutionState, WatcherInstance, WatcherState};
use serde::Serialize;
use std::sync::mpsc::{self, Receiver, SyncSender};
use std::sync::{Arc, Mutex};

/// Bounded snapshot notifications per subscriber before a slow consumer is
/// disconnected (TASK-0050): bounded so a stalled subscriber cannot grow
/// memory or stall the executor.
const SUBSCRIBER_BUFFER: usize = 16;

/// Looks up the frozen run-start estimate for one generation (TASK-0055):
/// wired from the duration recorder at the composition root; None for
/// non-target generations or when the surface is inactive.
pub type EstimateLookup = Arc<dyn Fn(u64) -> Option<RunEstimate> + Send + Sync>;

/// One consistent correlated snapshot (contract §7): instance + batch identity,
/// generation, per-task outcomes, pending work, and freshness tier. Field names
/// are camelCase and match the pi-watcher decoder and golden fixture.
#[derive(Clone, Debug, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct CorrelatedSnapshot {
    pub instance: WatcherInstance,
    pub generation: u64,
    pub batch_id: String,
    pub state: WatcherExecutionState,
    pub trigger: Option<String>,
    pub commands: Vec<String>,
    pub tasks: Vec<TaskSnapshot>,
    pub pending: u64,
    pub freshness: crate::awaiting::Freshness,
    pub duration_ms: Option<u64>,
    pub failures: Vec<String>,
    pub paths: Vec<String>,
    /// Frozen run-start duration estimate for the generation (TASK-0055,
    /// contract §6): present only for target runs with history; absent for
    /// legacy servers and non-target generations (omitted, never null).
    #[serde(skip_serializing_if = "Option::is_none")]
    pub estimate: Option<RunEstimate>,
    /// Configured scheduler concurrency of this watcher (TASK-0073): the
    /// bound from config, fixed per watcher session.
    pub configured_concurrency: usize,
    /// Effective concurrency of this generation (TASK-0073): one for a
    /// sequential override generation, otherwise the configured bound.
    pub effective_concurrency: usize,
    /// Override source label (TASK-0073): "control" for an exact control
    /// generation override; "config" otherwise. Omitted only for legacy
    /// servers that predate the field.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub concurrency_source: Option<String>,
    /// Immutable config revision this generation was frozen under (TASK-0089,
    /// CONFIG-RELOAD-CONTRACT §4). None for legacy servers that never
    /// observe reload; omitted when absent.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub revision: Option<u64>,
    /// Non-secret semantic hash of the frozen config revision (TASK-0089).
    #[serde(skip_serializing_if = "Option::is_none")]
    pub revision_hash: Option<String>,
    /// Live config lifecycle transition (TASK-0091, AC3/AC4): the current
    /// `configReloading`/`configReloaded`/`configInvalid` phase and its
    /// revision, read from the shared lifecycle source. Present only when a
    /// lifecycle source is wired; a reload transition publishes a snapshot
    /// even when the generation state is unchanged, so subscriptions receive
    /// the revision transition without reconnecting.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub config_lifecycle: Option<ConfigTransition>,
    /// Exact failure evidence with structured outputRef (TASK-0082, contract
    /// §1/§5): present only when the latest generation failed and retained
    /// output exists. Rendered identically by status, await, and subscribe.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub failure_evidence: Option<crate::output::FailureEvidence>,
}

struct Subscriber {
    sender: SyncSender<CorrelatedSnapshot>,
}

#[derive(Default)]
struct BrokerInner {
    subscribers: Vec<Subscriber>,
    /// Latest published snapshot; `subscribe` returns it as the immediate
    /// snapshot so registration and read share one lock with `publish`.
    last: Option<CorrelatedSnapshot>,
}

/// Shared subscription broker: one injected event publisher feeds `publish`,
/// and the control server's `subscribe` registers subscribers and returns the
/// immediate snapshot.
pub struct SnapshotBroker {
    instance: WatcherInstance,
    state: Arc<Mutex<WatcherState>>,
    coordinator: Arc<AwaitCoordinator>,
    /// Optional retained-output registry (TASK-0045): lets snapshots attach
    /// exact failure evidence with an outputRef. None keeps the legacy shape.
    outputs: Option<Arc<crate::output::OutputRegistry>>,
    /// Optional frozen run-start estimate lookup (TASK-0055): None keeps the
    /// legacy snapshot shape (no estimate key).
    estimates: Option<EstimateLookup>,
    /// Optional config lifecycle source (TASK-0091): None keeps the legacy
    /// snapshot shape (no configLifecycle key).
    lifecycle: std::sync::Mutex<Option<Arc<ConfigLifecycle>>>,
    /// Configured scheduler concurrency, fixed per watcher session
    /// (TASK-0073); reported verbatim on every snapshot.
    configured_concurrency: usize,
    inner: Mutex<BrokerInner>,
}

impl SnapshotBroker {
    pub fn new(
        instance: WatcherInstance,
        state: Arc<Mutex<WatcherState>>,
        coordinator: Arc<AwaitCoordinator>,
    ) -> Self {
        Self::with_estimates(instance, state, coordinator, None, 1)
    }

    /// Creates a broker that attaches the frozen run-start estimate to each
    /// snapshot for the current generation (TASK-0055). The lookup is wired
    /// from the duration recorder at the composition root.
    pub fn with_estimates(
        instance: WatcherInstance,
        state: Arc<Mutex<WatcherState>>,
        coordinator: Arc<AwaitCoordinator>,
        estimates: Option<EstimateLookup>,
        configured_concurrency: usize,
    ) -> Self {
        Self::with_outputs(
            instance,
            state,
            coordinator,
            None,
            estimates,
            configured_concurrency,
        )
    }

    /// Creates a broker that also attaches exact failure evidence with an
    /// outputRef (TASK-0082) when the latest generation failed and retained
    /// output exists.
    pub fn with_outputs(
        instance: WatcherInstance,
        state: Arc<Mutex<WatcherState>>,
        coordinator: Arc<AwaitCoordinator>,
        outputs: Option<Arc<crate::output::OutputRegistry>>,
        estimates: Option<EstimateLookup>,
        configured_concurrency: usize,
    ) -> Self {
        let broker = Self {
            instance,
            state,
            coordinator,
            outputs,
            estimates,
            lifecycle: std::sync::Mutex::new(None),
            configured_concurrency,
            inner: Mutex::new(BrokerInner::default()),
        };
        // Seed the idle snapshot so the first subscriber has an immediate value.
        let idle = broker.build();
        broker.inner.lock().unwrap().last = Some(idle);
        broker
    }

    /// Attaches the config lifecycle source (TASK-0091, AC3/AC4): snapshots
    /// then carry the live `configLifecycle` transition. The composition
    /// root separately registers a lifecycle watcher that calls `publish` on
    /// transitions, so a reload publishes even when generation state is
    /// unchanged (this method only records the source for snapshot reads).
    pub fn attach_lifecycle(&self, lifecycle: Arc<ConfigLifecycle>) {
        *self.lifecycle.lock().unwrap() = Some(lifecycle);
    }

    /// The instance identity the snapshot carries; shared with the control
    /// server so `capabilities` and snapshots report one token.
    pub fn instance(&self) -> &WatcherInstance {
        &self.instance
    }

    /// Rebuilds the snapshot from current state + coordinator facts.
    fn build(&self) -> CorrelatedSnapshot {
        let (latest_generation, _latest_batch, pending) = self.coordinator.snapshot_facts();
        let state = self.state.lock().unwrap().clone();
        let freshness = classify(&state, latest_generation, &pending);
        let estimate = self
            .estimates
            .as_ref()
            .and_then(|lookup| lookup(state.generation()));
        // Exact failure evidence (TASK-0082): only when the latest generation
        // failed and retained output exists; empty capture never emits a ref.
        let failure_evidence = if state.state() == &WatcherExecutionState::Failed {
            let failed_tasks: Vec<String> = state
                .tasks()
                .iter()
                .filter(|task| {
                    matches!(
                        task.state,
                        crate::executor::TaskState::Failed | crate::executor::TaskState::TimedOut
                    )
                })
                .map(|task| task.name.clone())
                .collect();
            self.outputs.as_ref().and_then(|outputs| {
                outputs.failure_evidence(
                    state.generation(),
                    DEFAULT_FAILURE_EVIDENCE_LINES,
                    &self.instance.token,
                    &failed_tasks,
                )
            })
        } else {
            None
        };
        CorrelatedSnapshot {
            instance: self.instance.clone(),
            generation: state.generation(),
            batch_id: state
                .batch()
                .map(|batch| format!("b-{}", batch))
                .unwrap_or_default(),
            state: state.state().clone(),
            trigger: state.trigger().map(str::to_owned),
            commands: state.commands().to_vec(),
            tasks: state.tasks().to_vec(),
            pending: pending.queued_batches as u64,
            freshness,
            duration_ms: state.duration_ms(),
            failures: state.failures().to_vec(),
            paths: state.changed().to_vec(),
            estimate,
            configured_concurrency: self.configured_concurrency,
            // TASK-0073: effective defaults to configured for generations
            // without an override; the source label rides the same state read.
            effective_concurrency: state
                .effective_concurrency()
                .unwrap_or(self.configured_concurrency),
            concurrency_source: Some(state.concurrency_source().unwrap_or("config").to_string()),
            revision: state.revision(),
            revision_hash: state.revision_hash().map(str::to_owned),
            config_lifecycle: self
                .lifecycle
                .lock()
                .unwrap()
                .as_ref()
                .map(|lifecycle| lifecycle.current()),
            failure_evidence,
        }
    }

    /// Publishes the current snapshot to every subscriber when it changed
    /// (whole-snapshot dedup, so unchanged state never emits a duplicate).
    /// Slow consumers whose bounded channel is full are disconnected.
    pub fn publish(&self) {
        let snapshot = self.build();
        let mut inner = self.inner.lock().unwrap();
        if inner.last.as_ref() == Some(&snapshot) {
            return;
        }
        inner.last = Some(snapshot.clone());
        inner
            .subscribers
            .retain(|subscriber| subscriber.sender.try_send(snapshot.clone()).is_ok());
    }

    /// Registers a subscriber and returns the current snapshot. Registration
    /// and read share the broker lock with `publish`, so no lifecycle
    /// transition can be lost between the read and listener registration.
    pub fn subscribe(&self) -> (Receiver<CorrelatedSnapshot>, CorrelatedSnapshot) {
        let (sender, receiver) = mpsc::sync_channel(SUBSCRIBER_BUFFER);
        let snapshot = {
            let mut inner = self.inner.lock().unwrap();
            inner.subscribers.push(Subscriber { sender });
            inner
                .last
                .clone()
                .expect("idle snapshot is seeded at construction")
        };
        (receiver, snapshot)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::awaiting::AwaitCoordinator;
    use crate::executor::Event;
    use std::time::Duration;

    fn broker() -> (
        Arc<SnapshotBroker>,
        Arc<Mutex<WatcherState>>,
        Arc<AwaitCoordinator>,
    ) {
        let state = Arc::new(Mutex::new(WatcherState::default()));
        let coordinator = Arc::new(AwaitCoordinator::new());
        let broker = Arc::new(SnapshotBroker::new(
            WatcherInstance {
                token: "fz-test".to_owned(),
                started_at_epoch_ms: 1,
            },
            Arc::clone(&state),
            Arc::clone(&coordinator),
        ));
        (broker, state, coordinator)
    }

    fn started(run_id: u64) -> Event {
        Event::Started {
            run_id,
            trigger: "src/main.rs".to_owned(),
            batch: Some(3),
            predecessor: None,
            changed: vec!["src/main.rs".to_owned()],
            commands: vec!["make all".to_owned()],
            target: None,
            execution_signature: None,
            effective_concurrency: None,
            concurrency_source: None,
            revision: None,
            revision_hash: None,
        }
    }

    #[test]
    fn subscribe_returns_the_idle_snapshot_with_shared_instance() {
        let (broker, _state, _coordinator) = broker();
        let (_rx, snapshot) = broker.subscribe();
        assert_eq!(snapshot.instance.token, "fz-test");
        assert_eq!(snapshot.generation, 0);
        assert_eq!(snapshot.state, WatcherExecutionState::Idle);
        assert!(snapshot.tasks.is_empty());
        assert_eq!(snapshot.batch_id, "");
    }

    #[test]
    fn publish_dedupes_unchanged_state() {
        let (broker, state, coordinator) = broker();
        let (rx, initial) = broker.subscribe();

        // First publish after a real transition must notify.
        coordinator.observe(&started(1));
        state.lock().unwrap().apply(started(1));
        broker.publish();
        let notified = rx.recv_timeout(Duration::from_millis(200)).unwrap();
        assert_eq!(notified.generation, 1);
        assert_eq!(notified.state, WatcherExecutionState::Running);
        assert_eq!(notified.batch_id, "b-3");
        assert_eq!(notified.paths, vec!["src/main.rs".to_owned()]);
        assert_ne!(notified, initial);

        // A second publish with no state change is a no-op.
        broker.publish();
        assert!(rx.try_recv().is_err(), "no duplicate for unchanged state");
    }

    #[test]
    fn multiple_subscribers_observe_the_same_transition() {
        let (broker, state, coordinator) = broker();
        let (rx1, _) = broker.subscribe();
        let (rx2, _) = broker.subscribe();

        coordinator.observe(&started(1));
        state.lock().unwrap().apply(started(1));
        broker.publish();

        let first = rx1.recv_timeout(Duration::from_millis(200)).unwrap();
        let second = rx2.recv_timeout(Duration::from_millis(200)).unwrap();
        assert_eq!(first, second);
        assert_eq!(first.generation, 1);
    }

    #[test]
    fn task_terminal_populates_the_snapshot_tasks() {
        let (broker, state, coordinator) = broker();
        let (rx, _) = broker.subscribe();

        coordinator.observe(&started(1));
        state.lock().unwrap().apply(started(1));
        state.lock().unwrap().apply(Event::TaskTerminal {
            run_id: 1,
            task: TaskSnapshot {
                position: 0,
                id: "checks#1".to_owned(),
                name: "check".to_owned(),
                state: crate::executor::TaskState::Passed,
                duration_ms: Some(42),
            },
        });
        broker.publish();

        let snapshot = rx.recv_timeout(Duration::from_millis(200)).unwrap();
        assert_eq!(snapshot.tasks.len(), 1);
        assert_eq!(snapshot.tasks[0].name, "check");
        assert_eq!(snapshot.tasks[0].duration_ms, Some(42));
    }

    #[test]
    fn snapshot_carries_frozen_estimate_only_when_lookup_provided() {
        use crate::duration_history::{EstimateConfidence, EstimateSource, RunEstimate};
        let state = Arc::new(Mutex::new(WatcherState::default()));
        let coordinator = Arc::new(AwaitCoordinator::new());
        state.lock().unwrap().apply(started(7));
        coordinator.observe(&started(7));

        let estimate = RunEstimate {
            typical_ms: 38_000,
            upper_ms: 61_000,
            recommended_timeout_ms: 95_000,
            samples: 12,
            confidence: EstimateConfidence::Medium,
            source: EstimateSource::Measured,
        };
        let lookup: EstimateLookup = Arc::new(move |run_id| {
            if run_id == 7 {
                Some(estimate.clone())
            } else {
                None
            }
        });

        // Without a lookup the snapshot has no estimate key at all.
        let plain = SnapshotBroker::new(
            WatcherInstance {
                token: "fz-test".to_owned(),
                started_at_epoch_ms: 1,
            },
            Arc::clone(&state),
            Arc::clone(&coordinator),
        );
        let (_, plain_snapshot) = plain.subscribe();
        let plain_json = serde_json::to_value(&plain_snapshot).unwrap();
        assert!(
            plain_json.get("estimate").is_none(),
            "legacy shape unchanged"
        );

        // With a lookup, the snapshot carries the estimate for the current
        // generation, camelCase.
        let with_estimates = SnapshotBroker::with_estimates(
            WatcherInstance {
                token: "fz-test".to_owned(),
                started_at_epoch_ms: 1,
            },
            Arc::clone(&state),
            Arc::clone(&coordinator),
            Some(lookup),
            2,
        );
        let (_, snapshot) = with_estimates.subscribe();
        let json = serde_json::to_value(&snapshot).unwrap();
        assert_eq!(json["estimate"]["typicalMs"], 38_000);
        assert_eq!(json["estimate"]["recommendedTimeoutMs"], 95_000);
        assert_eq!(json["estimate"]["confidence"], "medium");
        assert_eq!(json["estimate"]["source"], "measured");
        assert_eq!(json["configuredConcurrency"], 2);
        assert_eq!(json["effectiveConcurrency"], 2);
        assert_eq!(json["concurrencySource"], "config");
    }

    #[test]
    fn snapshot_carries_the_config_lifecycle_transition_when_attached() {
        // TASK-0091, AC3/AC4: after attach, the snapshot includes the live
        // config transition; a reload transition changes the snapshot even
        // when generation state is unchanged.
        let (broker, state, coordinator) = broker();
        state.lock().unwrap().apply(started(1));
        coordinator.observe(&started(1));
        state.lock().unwrap().apply(Event::Finished {
            run_id: 1,
            superseded_by: None,
            elapsed: Duration::from_millis(1),
            failures: vec![],
        });
        coordinator.observe(&Event::Finished {
            run_id: 1,
            superseded_by: None,
            elapsed: Duration::from_millis(1),
            failures: vec![],
        });
        broker.publish();

        let lifecycle = Arc::new(crate::config_lifecycle::ConfigLifecycle::new());
        broker.attach_lifecycle(Arc::clone(&lifecycle));
        // Composition-root wiring: the broker publishes on lifecycle
        // transitions (mirrors watch_non_block.rs).
        let broker_pub = Arc::clone(&broker);
        lifecycle.watch(Arc::new(move |_| broker_pub.publish()));
        // Refresh the broker's last snapshot so the immediate value carries
        // the attached lifecycle source.
        broker.publish();
        let (rx, _) = broker.subscribe();

        // Without a transition the snapshot carries the Idle transition.
        let (_, idle) = broker.subscribe();
        assert_eq!(
            idle.config_lifecycle.as_ref().map(|t| t.phase),
            Some(crate::config_lifecycle::ConfigPhase::Idle)
        );

        // A reloaded transition changes the snapshot and publishes.
        lifecycle.reloaded(&crate::config_revision::ConfigRevision {
            number: 2,
            hash: "hash-2".to_owned(),
        });
        let notified = rx.recv_timeout(Duration::from_millis(200)).unwrap();
        assert_eq!(
            notified.config_lifecycle.as_ref().map(|t| t.phase),
            Some(crate::config_lifecycle::ConfigPhase::ConfigReloaded)
        );
        assert_eq!(
            notified.config_lifecycle.as_ref().and_then(|t| t.revision),
            Some(2)
        );
        // The generation facts are unchanged: only the config transition moved.
        assert_eq!(notified.generation, 1);
    }

    #[test]
    fn snapshot_shape_matches_the_golden_fixture() {
        let state = Arc::new(Mutex::new(WatcherState::default()));
        let coordinator = Arc::new(AwaitCoordinator::new());
        state.lock().unwrap().apply(started(4));
        coordinator.observe(&started(4));
        state.lock().unwrap().apply(Event::TaskTerminal {
            run_id: 4,
            task: TaskSnapshot {
                position: 0,
                id: "t-1".to_owned(),
                name: "test @agent-final".to_owned(),
                state: crate::executor::TaskState::Passed,
                duration_ms: Some(42),
            },
        });
        state.lock().unwrap().apply(Event::Finished {
            run_id: 4,
            superseded_by: None,
            elapsed: Duration::from_millis(42),
            failures: vec![],
        });
        coordinator.observe(&Event::Finished {
            run_id: 4,
            superseded_by: None,
            elapsed: Duration::from_millis(42),
            failures: vec![],
        });

        let broker = SnapshotBroker::with_estimates(
            WatcherInstance {
                token: "fz-7f3a".to_owned(),
                started_at_epoch_ms: 1_710_000_000_000,
            },
            Arc::clone(&state),
            Arc::clone(&coordinator),
            None,
            2,
        );
        let (_, snapshot) = broker.subscribe();

        let json = serde_json::to_value(&snapshot).unwrap();
        assert_eq!(json["instance"]["token"], "fz-7f3a");
        assert_eq!(json["generation"], 4);
        assert_eq!(json["batchId"], "b-3");
        assert_eq!(json["state"], "passed");
        assert_eq!(json["trigger"], "src/main.rs");
        assert_eq!(json["commands"][0], "make all");
        assert_eq!(json["tasks"][0]["id"], "t-1");
        assert_eq!(json["tasks"][0]["name"], "test @agent-final");
        assert_eq!(json["tasks"][0]["state"], "passed");
        assert_eq!(json["tasks"][0]["durationMs"], 42);
        assert!(json["tasks"][0].get("position").is_none());
        assert_eq!(json["pending"], 0);
        assert_eq!(json["durationMs"], 42);
        assert!(json["failures"].as_array().unwrap().is_empty());
        assert_eq!(json["paths"][0], "src/main.rs");
        // TASK-0073 additive concurrency fields, matching the pi-watcher
        // golden fixture: configured 2, effective 2, source "config" for a
        // native run without any override.
        assert_eq!(json["configuredConcurrency"], 2);
        assert_eq!(json["effectiveConcurrency"], 2);
        assert_eq!(json["concurrencySource"], "config");
    }

    #[test]
    fn snapshot_reports_sequential_override_effective_concurrency() {
        let state = Arc::new(Mutex::new(WatcherState::default()));
        let coordinator = Arc::new(AwaitCoordinator::new());
        let mut started = started(5);
        if let Event::Started {
            effective_concurrency,
            concurrency_source,
            ..
        } = &mut started
        {
            *effective_concurrency = Some(1);
            *concurrency_source = Some("control");
        }
        let started_event = started.clone();
        state.lock().unwrap().apply(started);
        coordinator.observe(&started_event);

        let broker = SnapshotBroker::with_estimates(
            WatcherInstance {
                token: "fz-test".to_owned(),
                started_at_epoch_ms: 1,
            },
            Arc::clone(&state),
            Arc::clone(&coordinator),
            None,
            4,
        );
        let (_, snapshot) = broker.subscribe();
        let json = serde_json::to_value(&snapshot).unwrap();
        assert_eq!(json["configuredConcurrency"], 4);
        assert_eq!(json["effectiveConcurrency"], 1);
        assert_eq!(json["concurrencySource"], "control");
    }
}
