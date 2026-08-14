//! Correlated snapshot and subscription broker (TASK-0050, contract §7).
//!
//! The snapshot reuses the same injected event source as atomic await: it
//! reads `ControlState` (latest generation) plus the `AwaitCoordinator`
//! (pending-work and freshness facts) under the established lock order. The
//! broker owns subscribers and the bounded per-subscriber notification
//! channel; it never builds a second state tracker.

use crate::awaiting::{classify, AwaitCoordinator};
use crate::control::{ControlInstance, ControlState, ExecutionState};
use crate::executor::TaskSnapshot;
use serde_derive::Serialize;
use std::sync::mpsc::{self, Receiver, SyncSender};
use std::sync::{Arc, Mutex};

/// Bounded snapshot notifications per subscriber before a slow consumer is
/// disconnected (TASK-0050): bounded so a stalled subscriber cannot grow
/// memory or stall the executor.
const SUBSCRIBER_BUFFER: usize = 16;

/// One consistent correlated snapshot (contract §7): instance + batch identity,
/// generation, per-task outcomes, pending work, and freshness tier. Field names
/// are camelCase and match the pi-watcher decoder and golden fixture.
#[derive(Clone, Debug, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct CorrelatedSnapshot {
    pub instance: ControlInstance,
    pub generation: u64,
    pub batch_id: String,
    pub state: ExecutionState,
    pub trigger: Option<String>,
    pub commands: Vec<String>,
    pub tasks: Vec<TaskSnapshot>,
    pub pending: u64,
    pub freshness: crate::awaiting::Freshness,
    pub duration_ms: Option<u64>,
    pub failures: Vec<String>,
    pub paths: Vec<String>,
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
    instance: ControlInstance,
    state: Arc<Mutex<ControlState>>,
    coordinator: Arc<AwaitCoordinator>,
    inner: Mutex<BrokerInner>,
}

impl SnapshotBroker {
    pub fn new(
        instance: ControlInstance,
        state: Arc<Mutex<ControlState>>,
        coordinator: Arc<AwaitCoordinator>,
    ) -> Self {
        let broker = Self {
            instance,
            state,
            coordinator,
            inner: Mutex::new(BrokerInner::default()),
        };
        // Seed the idle snapshot so the first subscriber has an immediate value.
        let idle = broker.build();
        broker.inner.lock().unwrap().last = Some(idle);
        broker
    }

    /// The instance identity the snapshot carries; shared with the control
    /// server so `capabilities` and snapshots report one token.
    pub fn instance(&self) -> &ControlInstance {
        &self.instance
    }

    /// Rebuilds the snapshot from current state + coordinator facts.
    fn build(&self) -> CorrelatedSnapshot {
        let (latest_generation, _latest_batch, pending) = self.coordinator.snapshot_facts();
        let state = self.state.lock().unwrap().clone();
        let freshness = classify(&state, latest_generation, &pending);
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
    use crate::identity::Batch;
    use std::time::Duration;

    fn broker() -> (
        Arc<SnapshotBroker>,
        Arc<Mutex<ControlState>>,
        Arc<AwaitCoordinator>,
    ) {
        let state = Arc::new(Mutex::new(ControlState::default()));
        let coordinator = Arc::new(AwaitCoordinator::new());
        let broker = Arc::new(SnapshotBroker::new(
            ControlInstance {
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
        }
    }

    #[test]
    fn subscribe_returns_the_idle_snapshot_with_shared_instance() {
        let (broker, _state, _coordinator) = broker();
        let (_rx, snapshot) = broker.subscribe();
        assert_eq!(snapshot.instance.token, "fz-test");
        assert_eq!(snapshot.generation, 0);
        assert_eq!(snapshot.state, ExecutionState::Idle);
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
        assert_eq!(notified.state, ExecutionState::Running);
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
    fn snapshot_shape_matches_the_golden_fixture() {
        let state = Arc::new(Mutex::new(ControlState::default()));
        let coordinator = Arc::new(AwaitCoordinator::new());
        state.lock().unwrap().apply(started(4));
        coordinator.observe(&started(4));
        state.lock().unwrap().apply(Event::TaskTerminal {
            run_id: 4,
            task: TaskSnapshot {
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

        let broker = SnapshotBroker::new(
            ControlInstance {
                token: "fz-7f3a".to_owned(),
                started_at_epoch_ms: 1_710_000_000_000,
            },
            Arc::clone(&state),
            Arc::clone(&coordinator),
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
        assert_eq!(json["pending"], 0);
        assert_eq!(json["durationMs"], 42);
        assert!(json["failures"].as_array().unwrap().is_empty());
        assert_eq!(json["paths"][0], "src/main.rs");
    }
}
