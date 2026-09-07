//! Stable task-result contract shared by execution and presentation adapters.
//!
//! This module owns the serialized per-task result shape. It intentionally
//! depends only on serde, so executor implementation and stdout presentation
//! can depend on the contract without forming a cycle.

use serde::Serialize;

/// Wire-level task state for correlated snapshots and task-terminal events.
/// `Skipped` executor work is represented as `Cancelled`.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize)]
#[serde(rename_all = "lowercase")]
pub enum TaskState {
    Passed,
    Failed,
    Cancelled,
    /// Additive wire value distinct from command failure and client-await
    /// timeout.
    TimedOut,
}

/// One task's terminal outcome for correlated snapshots and reports.
#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct TaskSnapshot {
    /// Configured declaration position used for in-process report ordering;
    /// it is intentionally absent from the wire contract.
    #[serde(skip)]
    pub position: usize,
    pub id: String,
    pub name: String,
    pub state: TaskState,
    pub duration_ms: Option<u64>,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn contract_serializes_stable_wire_shape() {
        let snapshot = TaskSnapshot {
            position: 4,
            id: "checks#1".to_owned(),
            name: "lint".to_owned(),
            state: TaskState::TimedOut,
            duration_ms: Some(120),
        };
        let json = serde_json::to_value(snapshot).expect("task result serializes");
        assert_eq!(json["state"], "timedout");
        assert_eq!(json["durationMs"], 120);
        assert!(json.get("position").is_none());
    }
}
