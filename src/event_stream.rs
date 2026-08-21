//! NDJSON run-event stream (TASK-0039, contract docs/RUN-EVENTS-CONTRACT.md).
//!
//! Serializes the executor `Event` model into one JSON object per line to a
//! dedicated file, so agents and editors get a bounded machine-readable
//! stream without parsing human stdout. One write per line under a lock means
//! concurrent events are never byte-interleaved; a broken pipe disables the
//! sink with one warning and never fails the run.

use crate::executor::{Event, EventSink};
use crate::stdout;
use serde_json::{json, Value};
use std::fs::{File, OpenOptions};
use std::io::{BufWriter, Write};
use std::path::Path;
use std::sync::Mutex;
use std::time::{SystemTime, UNIX_EPOCH};

/// Schema version of the NDJSON event records (contract §3).
pub const EVENT_SCHEMA_VERSION: u64 = 1;

/// Appends NDJSON run events to one file. A single `Mutex` serializes
/// writers so every record is one atomic line (no interleaving); after a
/// write failure the sink warns once and disables itself.
pub struct EventStream {
    writer: Mutex<Option<BufWriter<File>>>,
}

impl EventStream {
    /// Opens the stream in append mode; missing parent directories are a
    /// hard error (caller decides how to present it).
    pub fn open(path: &Path) -> std::io::Result<Self> {
        let file = OpenOptions::new().create(true).append(true).open(path)?;
        Ok(Self {
            writer: Mutex::new(Some(BufWriter::new(file))),
        })
    }

    /// Convenience inherent wrapper so callers do not need the trait in
    /// scope to append one event.
    pub fn emit_event(&self, event: Event) {
        EventSink::emit(self, event);
    }

    /// Whether the sink is still active (write failures disable it).
    pub fn is_active(&self) -> bool {
        self.writer.lock().map(|w| w.is_some()).unwrap_or(false)
    }

    /// Serializes one executor event into a contract record (schema version,
    /// kind, run identity, and kind-specific fields).
    fn serialize(event: &Event) -> Value {
        let ts_ms = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .map(|d| d.as_millis() as u64)
            .unwrap_or(0);
        match event {
            Event::Started {
                run_id,
                trigger,
                batch,
                predecessor,
                changed,
                commands,
                target,
                effective_concurrency,
                concurrency_source,
                ..
            } => json!({
                "schemaVersion": EVENT_SCHEMA_VERSION,
                "event": "started",
                "runId": run_id,
                "tsMs": ts_ms,
                "trigger": trigger,
                "batch": batch,
                "predecessor": predecessor,
                "changed": changed,
                "commands": commands,
                "target": target,
                "effectiveConcurrency": effective_concurrency,
                "concurrencySource": concurrency_source,
            }),
            Event::Tick {
                task,
                group_occurrence,
                ..
            } => json!({
                "schemaVersion": EVENT_SCHEMA_VERSION,
                "event": "tick",
                "runId": null,
                "tsMs": ts_ms,
                "task": task,
                "group": group_occurrence,
            }),
            Event::TaskTerminal { run_id, task } => json!({
                "schemaVersion": EVENT_SCHEMA_VERSION,
                "event": "task_terminal",
                "runId": run_id,
                "tsMs": ts_ms,
                "task": task.name,
                "group": task.id,
                "state": match task.state {
                    crate::executor::TaskState::Passed => "passed",
                    crate::executor::TaskState::Failed => "failed",
                    crate::executor::TaskState::Cancelled => "cancelled",
                },
                "durationMs": task.duration_ms,
            }),
            Event::Finished {
                run_id,
                superseded_by,
                elapsed,
                failures,
            } => json!({
                "schemaVersion": EVENT_SCHEMA_VERSION,
                "event": "finished",
                "runId": run_id,
                "tsMs": ts_ms,
                "elapsedMs": elapsed.as_millis() as u64,
                "failures": failures,
                "supersededBy": superseded_by,
            }),
            Event::Cancelled {
                run_id,
                superseded_by,
            } => json!({
                "schemaVersion": EVENT_SCHEMA_VERSION,
                "event": "cancelled",
                "runId": run_id,
                "tsMs": ts_ms,
                "supersededBy": superseded_by,
            }),
            Event::RecoveryPhase {
                run_id,
                job,
                phase,
                outcome,
            } => json!({
                "schemaVersion": EVENT_SCHEMA_VERSION,
                "event": "recovery_phase",
                "runId": run_id,
                "tsMs": ts_ms,
                "job": job,
                "phase": phase,
                "outcome": outcome,
            }),
        }
    }
}

impl EventSink for EventStream {
    fn emit(&self, event: Event) {
        let mut guard = match self.writer.lock() {
            Ok(guard) => guard,
            Err(_) => return,
        };
        let Some(writer) = guard.as_mut() else {
            return;
        };
        let record = Self::serialize(&event);
        let mut line = serde_json::to_string(&record).unwrap_or_default();
        line.push('\n');
        let result = writer
            .write_all(line.as_bytes())
            .and_then(|_| writer.flush());
        if let Err(err) = result {
            // Broken pipe or disk failure: warn once, disable, never fail
            // the run (contract §5).
            stdout::warn(&format!("run event stream disabled: {}", err));
            *guard = None;
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::executor::TaskSnapshot;
    use crate::plan::ExecutionSignature;
    use std::time::Duration;

    fn stream_in_temp(name: &str) -> (std::path::PathBuf, EventStream) {
        let path =
            std::env::temp_dir().join(format!("funzzy-events-{}-{}", std::process::id(), name));
        let _ = std::fs::remove_file(&path);
        let stream = EventStream::open(&path).expect("open stream");
        (path, stream)
    }

    fn read_lines(path: &std::path::Path) -> Vec<Value> {
        let content = std::fs::read_to_string(path).expect("read stream");
        content
            .lines()
            .map(|line| serde_json::from_str(line).expect("valid ndjson line"))
            .collect()
    }

    fn started() -> Event {
        Event::Started {
            run_id: 7,
            trigger: "src/main.rs".to_owned(),
            batch: Some(3),
            predecessor: None,
            changed: vec!["src/main.rs".to_owned()],
            commands: vec!["cargo test".to_owned()],
            target: Some("tests".to_owned()),
            execution_signature: Some(ExecutionSignature("sig-1".to_owned())),
            effective_concurrency: Some(2),
            concurrency_source: Some("config"),
            revision: None,
            revision_hash: None,
        }
    }

    #[test]
    fn started_record_carries_schema_kind_and_identity() {
        let (path, stream) = stream_in_temp("started");
        stream.emit(started());
        drop(stream);
        let records = read_lines(&path);
        assert_eq!(records.len(), 1);
        assert_eq!(records[0]["schemaVersion"], 1);
        assert_eq!(records[0]["event"], "started");
        assert_eq!(records[0]["runId"], 7);
        assert_eq!(records[0]["batch"], 3);
        assert_eq!(records[0]["target"], "tests");
        assert_eq!(records[0]["effectiveConcurrency"], 2);
        assert_eq!(records[0]["concurrencySource"], "config");
        assert!(records[0]["tsMs"].is_number());
        std::fs::remove_file(&path).ok();
    }

    #[test]
    fn finished_record_is_final_and_order_independent() {
        let (path, stream) = stream_in_temp("finished");
        stream.emit(Event::Finished {
            run_id: 7,
            superseded_by: None,
            elapsed: Duration::from_millis(42),
            failures: vec!["b: boom".to_owned(), "a: crash".to_owned()],
        });
        drop(stream);
        let records = read_lines(&path);
        assert_eq!(records[0]["event"], "finished");
        assert_eq!(records[0]["elapsedMs"], 42);
        assert_eq!(records[0]["failures"][0], "b: boom");
        assert_eq!(records[0]["supersededBy"], Value::Null);
        std::fs::remove_file(&path).ok();
    }

    #[test]
    fn task_terminal_carries_task_and_group_identity() {
        let (path, stream) = stream_in_temp("task-terminal");
        stream.emit(Event::TaskTerminal {
            run_id: 7,
            task: TaskSnapshot {
                position: 0,
                id: "checks#1".to_owned(),
                name: "test @quick".to_owned(),
                state: crate::executor::TaskState::Passed,
                duration_ms: Some(120),
            },
        });
        drop(stream);
        let records = read_lines(&path);
        assert_eq!(records[0]["event"], "task_terminal");
        assert_eq!(records[0]["runId"], 7);
        assert_eq!(records[0]["task"], "test @quick");
        assert_eq!(records[0]["group"], "checks#1");
        assert_eq!(records[0]["state"], "passed");
        assert_eq!(records[0]["durationMs"], 120);
        std::fs::remove_file(&path).ok();
    }

    #[test]
    fn recovery_phase_carries_structured_phase_and_outcome() {
        let (path, stream) = stream_in_temp("recovery-phase");
        stream.emit(Event::RecoveryPhase {
            run_id: 7,
            job: "format @quick".to_owned(),
            phase: "verification_finished".to_owned(),
            outcome: Some("passed".to_owned()),
        });
        drop(stream);
        let records = read_lines(&path);
        assert_eq!(records[0]["event"], "recovery_phase");
        assert_eq!(records[0]["runId"], 7);
        assert_eq!(records[0]["job"], "format @quick");
        assert_eq!(records[0]["phase"], "verification_finished");
        assert_eq!(records[0]["outcome"], "passed");
        std::fs::remove_file(&path).ok();
    }

    #[test]
    fn cancelled_and_superseded_are_distinct_records() {
        let (path, stream) = stream_in_temp("cancelled");
        stream.emit(Event::Cancelled {
            run_id: 9,
            superseded_by: Some(10),
        });
        drop(stream);
        let records = read_lines(&path);
        assert_eq!(records[0]["event"], "cancelled");
        assert_eq!(records[0]["runId"], 9);
        assert_eq!(records[0]["supersededBy"], 10);
        std::fs::remove_file(&path).ok();
    }

    #[test]
    fn concurrent_events_are_line_atomic() {
        use std::sync::Arc;
        let (path, stream) = stream_in_temp("concurrent");
        let stream = Arc::new(stream);
        let mut handles = vec![];
        for i in 0..8 {
            let stream = Arc::clone(&stream);
            handles.push(std::thread::spawn(move || {
                for _ in 0..50 {
                    stream.emit(Event::Tick {
                        task: format!("task-{i}"),
                        group_occurrence: Some("g#1".to_owned()),
                    });
                }
            }));
        }
        for handle in handles {
            handle.join().unwrap();
        }
        drop(stream);
        let records = read_lines(&path);
        assert_eq!(records.len(), 400);
        for record in &records {
            assert_eq!(record["event"], "tick");
            assert!(record["task"].is_string());
        }
        std::fs::remove_file(&path).ok();
    }
}
