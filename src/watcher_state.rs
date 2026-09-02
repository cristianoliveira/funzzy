//! User-facing watcher execution state.
//!
//! Projects executor events into one coherent latest-generation view shared by
//! awaiting, snapshots, and control transport. This module owns no socket or
//! JSON-RPC behavior.

use crate::executor::{Event, TaskSnapshot};
use serde::Serialize;

#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
#[serde(rename_all = "lowercase")]
pub enum WatcherExecutionState {
    Idle,
    Running,
    Passed,
    Failed,
    Cancelled,
}

/// One Funzzy process identity: token changes on restart so clients can detect
/// instance changes instead of assuming continuity.
#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct WatcherInstance {
    pub token: String,
    pub started_at_epoch_ms: u64,
}

impl Default for WatcherInstance {
    fn default() -> Self {
        Self::new()
    }
}

impl WatcherInstance {
    pub fn new() -> Self {
        let started_at_epoch_ms = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map(|duration| duration.as_millis() as u64)
            .unwrap_or(0);
        let nanos = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map(|duration| duration.as_nanos())
            .unwrap_or(0);
        let token = format!("fz-{:016x}{:08x}", nanos, std::process::id());
        Self {
            token,
            started_at_epoch_ms,
        }
    }
}

#[derive(Clone, Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct WatcherState {
    generation: u64,
    state: WatcherExecutionState,
    trigger: Option<String>,
    commands: Vec<String>,
    duration_ms: Option<u64>,
    failures: Vec<String>,
    /// Correlation fields belong to same generation and are updated under one
    /// lock, so one read never mixes generations.
    batch: Option<u64>,
    changed: Vec<String>,
    predecessor: Option<u64>,
    superseded_by: Option<u64>,
    /// Per-task terminal outcomes are additive control data. `TaskSnapshot`
    /// keeps its internal declaration position serde-skipped, preserving the
    /// existing `tasks[].durationMs` wire shape.
    tasks: Vec<TaskSnapshot>,
    effective_concurrency: Option<usize>,
    concurrency_source: Option<&'static str>,
    /// Immutable configuration revision this generation was frozen under.
    revision: Option<u64>,
    revision_hash: Option<String>,
}

impl WatcherState {
    pub fn is_running(&self) -> bool {
        self.state == WatcherExecutionState::Running
    }
}

impl Default for WatcherState {
    fn default() -> Self {
        Self {
            generation: 0,
            state: WatcherExecutionState::Idle,
            trigger: None,
            commands: vec![],
            duration_ms: None,
            failures: vec![],
            batch: None,
            changed: vec![],
            predecessor: None,
            superseded_by: None,
            tasks: vec![],
            effective_concurrency: None,
            concurrency_source: None,
            revision: None,
            revision_hash: None,
        }
    }
}

impl WatcherState {
    pub fn generation(&self) -> u64 {
        self.generation
    }

    pub fn state(&self) -> &WatcherExecutionState {
        &self.state
    }

    pub fn predecessor(&self) -> Option<u64> {
        self.predecessor
    }

    pub fn superseded_by(&self) -> Option<u64> {
        self.superseded_by
    }

    pub fn tasks(&self) -> &[TaskSnapshot] {
        &self.tasks
    }

    pub fn trigger(&self) -> Option<&str> {
        self.trigger.as_deref()
    }

    pub fn commands(&self) -> &[String] {
        &self.commands
    }

    pub fn duration_ms(&self) -> Option<u64> {
        self.duration_ms
    }

    pub fn failures(&self) -> &[String] {
        &self.failures
    }

    pub fn revision(&self) -> Option<u64> {
        self.revision
    }

    pub fn revision_hash(&self) -> Option<&str> {
        self.revision_hash.as_deref()
    }

    pub fn batch(&self) -> Option<u64> {
        self.batch
    }

    pub fn changed(&self) -> &[String] {
        &self.changed
    }

    pub fn effective_concurrency(&self) -> Option<usize> {
        self.effective_concurrency
    }

    pub fn concurrency_source(&self) -> Option<&'static str> {
        self.concurrency_source
    }

    pub fn apply(&mut self, event: Event) {
        match event {
            Event::Started {
                run_id,
                trigger,
                batch,
                predecessor,
                changed,
                commands,
                effective_concurrency,
                concurrency_source,
                revision,
                revision_hash,
                ..
            } => {
                self.generation = run_id;
                self.state = WatcherExecutionState::Running;
                self.trigger = Some(trigger);
                self.batch = batch;
                self.changed = changed;
                self.predecessor = predecessor;
                self.superseded_by = None;
                self.commands = commands;
                self.duration_ms = None;
                self.failures.clear();
                self.tasks.clear();
                self.effective_concurrency = effective_concurrency;
                self.concurrency_source = concurrency_source;
                self.revision = revision;
                self.revision_hash = revision_hash;
            }
            Event::Finished {
                superseded_by,
                elapsed,
                failures,
                ..
            } => {
                self.state = if failures.is_empty() {
                    WatcherExecutionState::Passed
                } else {
                    WatcherExecutionState::Failed
                };
                self.duration_ms = Some(elapsed.as_millis() as u64);
                self.failures = failures;
                self.superseded_by = superseded_by;
            }
            Event::Cancelled { superseded_by, .. } => {
                self.state = WatcherExecutionState::Cancelled;
                self.duration_ms = None;
                self.superseded_by = superseded_by;
            }
            Event::Tick { .. } => {}
            Event::TaskTerminal { run_id, task } => {
                if run_id == self.generation {
                    self.tasks.push(task);
                    self.tasks.sort_by_key(|task| task.position);
                }
            }
            Event::RecoveryPhase { .. } => {}
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::time::Duration;

    fn started(run_id: u64, batch: Option<u64>, predecessor: Option<u64>) -> Event {
        Event::Started {
            run_id,
            trigger: "src/main.rs".to_owned(),
            batch,
            predecessor,
            changed: vec!["src/main.rs".to_owned()],
            commands: vec!["echo hi".to_owned()],
            target: None,
            execution_signature: None,
            effective_concurrency: None,
            concurrency_source: None,
            revision: Some(3),
            revision_hash: Some("abc123".to_owned()),
        }
    }

    #[test]
    fn started_sets_all_correlation_fields_coherently() {
        let mut state = WatcherState::default();
        state.apply(started(42, Some(7), Some(41)));

        assert_eq!(state.generation(), 42);
        assert_eq!(state.state(), &WatcherExecutionState::Running);
        assert_eq!(state.trigger(), Some("src/main.rs"));
        assert_eq!(state.batch(), Some(7));
        assert_eq!(state.changed(), ["src/main.rs"]);
        assert_eq!(state.predecessor(), Some(41));
        assert_eq!(state.superseded_by(), None);
        assert_eq!(state.duration_ms(), None);
        assert!(state.failures().is_empty());
        assert_eq!(state.revision(), Some(3));
        assert_eq!(state.revision_hash(), Some("abc123"));
    }

    #[test]
    fn finished_records_terminal_state_and_superseded_relation() {
        let mut state = WatcherState::default();
        state.apply(started(42, None, None));
        state.apply(Event::Finished {
            run_id: 42,
            superseded_by: Some(43),
            elapsed: Duration::from_millis(9),
            failures: vec!["boom".to_owned()],
        });

        assert_eq!(state.state(), &WatcherExecutionState::Failed);
        assert_eq!(state.duration_ms(), Some(9));
        assert_eq!(state.failures(), ["boom"]);
        assert_eq!(state.superseded_by(), Some(43));
        assert_eq!(state.generation(), 42);
        assert_eq!(state.revision(), Some(3));
        assert_eq!(state.revision_hash(), Some("abc123"));
    }

    #[test]
    fn cancelled_records_generation_and_superseded_by() {
        let mut state = WatcherState::default();
        state.apply(started(1, None, None));
        state.apply(Event::Cancelled {
            run_id: 1,
            superseded_by: Some(2),
        });

        assert_eq!(state.state(), &WatcherExecutionState::Cancelled);
        assert_eq!(state.superseded_by(), Some(2));
        assert_eq!(state.duration_ms(), None);
        assert_eq!(state.generation(), 1);
    }

    #[test]
    fn replacement_transition_keeps_latest_generation_consistent() {
        let mut state = WatcherState::default();
        state.apply(started(1, None, None));
        state.apply(Event::Cancelled {
            run_id: 1,
            superseded_by: Some(2),
        });
        state.apply(started(2, Some(5), Some(1)));

        assert_eq!(state.generation(), 2);
        assert_eq!(state.batch(), Some(5));
        assert_eq!(state.predecessor(), Some(1));
        assert_eq!(state.superseded_by(), None);
        assert_eq!(state.state(), &WatcherExecutionState::Running);
    }

    #[test]
    fn task_snapshots_sort_by_executor_declaration_position() {
        let mut state = WatcherState::default();
        state.apply(started(1, None, None));
        for (position, name) in [(1, "second"), (0, "first")] {
            state.apply(Event::TaskTerminal {
                run_id: 1,
                task: TaskSnapshot {
                    position,
                    id: name.to_owned(),
                    name: name.to_owned(),
                    state: crate::executor::TaskState::Passed,
                    duration_ms: Some(42),
                },
            });
        }

        assert_eq!(
            state
                .tasks()
                .iter()
                .map(|task| task.name.as_str())
                .collect::<Vec<_>>(),
            ["first", "second"]
        );
    }

    #[test]
    fn status_serializes_ordered_task_snapshots_without_internal_position() {
        let mut state = WatcherState::default();
        state.apply(started(1, None, None));
        for (position, name, duration_ms) in [(1, "second", None), (0, "first", Some(42))] {
            state.apply(Event::TaskTerminal {
                run_id: 1,
                task: TaskSnapshot {
                    position,
                    id: name.to_owned(),
                    name: name.to_owned(),
                    state: crate::executor::TaskState::Passed,
                    duration_ms,
                },
            });
        }

        let json = serde_json::to_value(state).unwrap();
        assert_eq!(json["tasks"][0]["name"], "first");
        assert_eq!(json["tasks"][0]["durationMs"], 42);
        assert_eq!(json["tasks"][1]["durationMs"], serde_json::Value::Null);
        assert!(json["tasks"][0].get("position").is_none());
    }

    #[test]
    fn managed_service_event_updates_live_services_without_rewriting_generation() {
        let mut state = WatcherState::default();
        state.apply(started(42, None, None));
        state.apply(Event::ServiceLifecycle {
            sequence: 1,
            ts_ms: 100,
            service: crate::service_pool::ManagedServiceSnapshot {
                name: "api".to_owned(),
                instance_id: 7,
                state: crate::service_pool::ServiceState::Ready,
                origin_generation: Some(42),
                revision: 3,
                signature: "api-v3".to_owned(),
                restart_attempts_used: 0,
                restart_attempts_remaining: 3,
                started_at_epoch_ms: Some(90),
                ready_at_epoch_ms: Some(100),
                uptime_ms: Some(10),
                latest_error: None,
            },
        });

        assert_eq!(state.generation(), 42);
        assert_eq!(state.state(), &WatcherExecutionState::Running);
        assert_eq!(state.services()[0].name, "api");
        assert_eq!(state.services()[0].state, crate::service_pool::ServiceState::Ready);
        assert_eq!(serde_json::to_value(&state).unwrap()["services"][0]["instanceId"], 7);
    }

    #[test]
    fn default_status_always_contains_an_empty_services_array() {
        let value = serde_json::to_value(WatcherState::default()).unwrap();
        assert_eq!(value["services"], serde_json::json!([]));
    }

    #[test]
    fn legacy_fields_serialize_verbatim_with_additive_correlation_keys() {
        let mut state = WatcherState::default();
        state.apply(started(42, Some(7), None));
        let json = serde_json::to_value(state).unwrap();
        let object = json.as_object().unwrap();

        assert_eq!(object["generation"], serde_json::json!(42));
        assert_eq!(object["state"], serde_json::json!("running"));
        assert_eq!(object["trigger"], serde_json::json!("src/main.rs"));
        assert!(object.contains_key("commands"));
        assert!(object.contains_key("durationMs"));
        assert!(object.contains_key("failures"));
        assert_eq!(object["batch"], serde_json::json!(7));
        assert_eq!(object["changed"], serde_json::json!(["src/main.rs"]));
        assert_eq!(object["predecessor"], serde_json::json!(null));
        assert_eq!(object["supersededBy"], serde_json::json!(null));
    }
}
