//! Duration recorder: projects executor terminal events into bounded
//! duration history and persists it (TASK-0054).
//!
//! The recorder is persistence- and statistics-agnostic wiring: it consumes
//! [`Event`]s (only target runs, identified structurally via
//! `Event::Started.target`/`execution_signature`), maps run identity to its
//! profile, records the terminal wall duration, and best-effort persists
//! through [`DurationStore`]. Executor and estimator stay unaware of each
//! other; a persistence failure emits a concise warning and never changes
//! the workflow result or blocks event delivery.

use crate::duration_history::{DurationHistory, ExcludedKind, RunEstimate};
use crate::duration_store::DurationStore;
use crate::executor::Event;
use crate::plan::ExecutionSignature;
use crate::stdout;
use std::collections::BTreeMap;
use std::sync::Mutex;

/// Upper bound on in-flight run→profile associations. The worker schedules
/// at most a bounded number of queued/running generations, so this is a
/// defensive ceiling; associations are removed at terminal state.
const MAX_ASSOCIATIONS: usize = 64;
/// Upper bound on frozen run-start estimates retained for terminal snapshots
/// (TASK-0055): terminal snapshots must still show the estimate captured when
/// the generation started, so captured values outlive the association.
const MAX_CAPTURED_ESTIMATES: usize = 256;

/// One in-flight run: the profile identity to record against. The frozen
/// run-start estimate lives in `captured_estimates` so it survives terminal.
struct Association {
    signature: ExecutionSignature,
}

/// Projects terminal events into duration history and persists it.
pub struct DurationRecorder {
    history: Mutex<DurationHistory>,
    store: DurationStore,
    /// run_id -> profile while queued/running; removed at terminal state so
    /// the map stays bounded and duplicate terminal events are no-ops.
    associations: Mutex<BTreeMap<u64, Association>>,
    /// run_id -> frozen run-start estimate, retained past terminal so the
    /// terminal snapshot still carries it (TASK-0055); bounded, oldest-first.
    captured_estimates: Mutex<BTreeMap<u64, RunEstimate>>,
}

impl DurationRecorder {
    /// Creates a recorder seeded from the store (empty when missing or
    /// unrecoverable). A recovery warning is surfaced once; the watcher
    /// stays usable.
    pub fn new(store: DurationStore) -> Self {
        let outcome = store.load();
        if let Some(warning) = outcome.warning {
            stdout::warn(&warning);
        }
        Self {
            history: Mutex::new(outcome.history),
            store,
            associations: Mutex::new(BTreeMap::new()),
            captured_estimates: Mutex::new(BTreeMap::new()),
        }
    }

    /// Consumes one executor event. Only target runs (carrying a structural
    /// execution signature) are recorded; filesystem/init/emit runs without a
    /// signature are ignored, so they can never contaminate target history.
    pub fn observe(&self, event: &Event) {
        match event {
            Event::Started {
                run_id,
                execution_signature: Some(signature),
                ..
            } => {
                let mut associations = self.associations.lock().unwrap();
                if associations.len() >= MAX_ASSOCIATIONS {
                    // Defensive bound: evict the oldest in-flight association
                    // rather than growing without limit.
                    if let Some((oldest, _)) = associations.pop_first() {
                        stdout::warn(&format!(
                            "funzzy: duration association bound reached; dropping run {}",
                            oldest
                        ));
                    }
                }
                // Freeze the estimate at run start: snapshot the current
                // history before this run can change it (contract §6).
                if let Some(estimate) = self.history.lock().unwrap().estimate(signature, None) {
                    let mut captured = self.captured_estimates.lock().unwrap();
                    if captured.len() >= MAX_CAPTURED_ESTIMATES {
                        if let Some((oldest, _)) = captured.pop_first() {
                            stdout::warn(&format!(
                                "funzzy: captured estimate bound reached; dropping run {}",
                                oldest
                            ));
                        }
                    }
                    captured.insert(*run_id, estimate);
                }
                associations.insert(
                    *run_id,
                    Association {
                        signature: signature.clone(),
                    },
                );
            }
            Event::Finished {
                run_id,
                superseded_by,
                elapsed,
                failures,
                ..
            } => {
                let association = self.associations.lock().unwrap().remove(run_id);
                let Some(association) = association else {
                    return; // duplicate terminal or non-target run: no-op
                };
                let duration_ms = elapsed.as_millis() as u64;
                let mut history = self.history.lock().unwrap();
                if superseded_by.is_some() {
                    // A superseded generation is never reported as passed or
                    // failed (contract §1); count it excluded.
                    history.record_excluded(&association.signature, ExcludedKind::Superseded);
                } else if failures.is_empty() {
                    history.record_success(&association.signature, duration_ms);
                } else {
                    history.record_failure(&association.signature, duration_ms);
                }
                drop(history);
                self.persist_best_effort();
            }
            Event::Cancelled {
                run_id,
                superseded_by,
                ..
            } => {
                let association = self.associations.lock().unwrap().remove(run_id);
                let Some(association) = association else {
                    return;
                };
                let kind = if superseded_by.is_some() {
                    ExcludedKind::Superseded
                } else {
                    ExcludedKind::Cancelled
                };
                let mut history = self.history.lock().unwrap();
                history.record_excluded(&association.signature, kind);
                drop(history);
                self.persist_best_effort();
            }
            // Tick and TaskTerminal carry no run-level terminal outcome.
            _ => {}
        }
    }

    /// Classifies an in-flight run as timed out (observer bound, contract
    /// §1): the run itself is not cancelled, but its eventual terminal event
    /// records an excluded timed-out outcome instead of success/failure. The
    /// association is removed so the later terminal event is a no-op.
    pub fn note_timeout(&self, run_id: u64) {
        let association = self.associations.lock().unwrap().remove(&run_id);
        if let Some(association) = association {
            let mut history = self.history.lock().unwrap();
            history.record_excluded(&association.signature, ExcludedKind::TimedOut);
            drop(history);
            self.persist_best_effort();
        }
    }

    /// Derives the current estimate for a signature, if history exists.
    pub fn estimate(
        &self,
        signature: &ExecutionSignature,
        configured_floor_ms: Option<u64>,
    ) -> Option<RunEstimate> {
        self.history
            .lock()
            .unwrap()
            .estimate(signature, configured_floor_ms)
    }

    /// Estimate captured at run start for a generation (TASK-0055): the value
    /// is frozen when the target generation starts and never re-derived as
    /// history changes mid-run, so progress fields stay stable for the whole
    /// generation (contract §6). None for non-target generations.
    pub fn estimate_at_start(&self, run_id: u64) -> Option<RunEstimate> {
        self.captured_estimates
            .lock()
            .unwrap()
            .get(&run_id)
            .cloned()
    }

    /// Number of recorded success samples for a signature (tests/diagnostics).
    pub fn success_samples(&self, signature: &ExecutionSignature) -> usize {
        self.history.lock().unwrap().success_samples(signature)
    }

    /// In-flight association count (tests/diagnostics).
    pub fn in_flight(&self) -> usize {
        self.associations.lock().unwrap().len()
    }

    /// Best-effort persist: a failure emits one concise warning and never
    /// propagates, so recording cannot change the workflow result or deadlock
    /// event delivery (contract §5).
    fn persist_best_effort(&self) {
        let history = self.history.lock().unwrap().clone();
        if let Err(error) = self.store.save(&history) {
            stdout::warn(&format!(
                "funzzy: cannot persist duration history: {}",
                error
            ));
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::duration_store::DurationStore;
    use crate::executor::Event;
    use std::sync::atomic::{AtomicU64, Ordering};
    use std::time::Duration;

    fn sig(n: u64) -> ExecutionSignature {
        ExecutionSignature(format!("sig-{n}"))
    }

    fn started(run_id: u64, target: Option<&str>, signature: Option<u64>) -> Event {
        Event::Started {
            run_id,
            trigger: "ignored".to_owned(),
            batch: None,
            predecessor: None,
            changed: vec![],
            commands: vec!["make all".to_owned()],
            target: target.map(str::to_owned),
            execution_signature: signature.map(sig),
        }
    }

    fn finished(run_id: u64, failures: bool, superseded_by: Option<u64>) -> Event {
        Event::Finished {
            run_id,
            superseded_by,
            elapsed: Duration::from_millis(40_000),
            failures: if failures {
                vec!["boom".to_owned()]
            } else {
                vec![]
            },
        }
    }

    fn cancelled(run_id: u64, superseded_by: Option<u64>) -> Event {
        Event::Cancelled {
            run_id,
            superseded_by,
        }
    }

    struct TempDir(std::path::PathBuf);
    static TEMP_SEQ: AtomicU64 = AtomicU64::new(0);

    impl TempDir {
        fn new() -> Self {
            let seq = TEMP_SEQ.fetch_add(1, Ordering::Relaxed);
            let dir =
                std::env::temp_dir().join(format!("funzzy-recorder-{}-{seq}", std::process::id()));
            let _ = std::fs::remove_dir_all(&dir);
            std::fs::create_dir_all(&dir).unwrap();
            Self(dir)
        }
    }

    impl Drop for TempDir {
        fn drop(&mut self) {
            let _ = std::fs::remove_dir_all(&self.0);
        }
    }

    fn recorder() -> (DurationRecorder, TempDir) {
        let temp = TempDir::new();
        // Direct temp path: never resolve via XDG/HOME env, so recorder tests
        // cannot race with the env-mutation tests in duration_store.
        let store = DurationStore::new(temp.0.join("run-durations-v1.json"));
        (DurationRecorder::new(store), temp)
    }

    #[test]
    fn passed_target_records_one_success_sample() {
        let (recorder, _temp) = recorder();
        let signature = sig(1);
        recorder.observe(&started(1, Some("build"), Some(1)));
        recorder.observe(&finished(1, false, None));
        assert_eq!(recorder.success_samples(&signature), 1);
        assert_eq!(recorder.in_flight(), 0);
        let estimate = recorder.estimate(&signature, None).unwrap();
        assert_eq!(estimate.typical_ms, 40_000);
        assert_eq!(estimate.upper_ms, 40_000);
    }

    #[test]
    fn failed_target_records_separate_failure_outcome() {
        let (recorder, _temp) = recorder();
        let signature = sig(1);
        recorder.observe(&started(1, Some("build"), Some(1)));
        recorder.observe(&finished(1, true, None));
        assert_eq!(recorder.success_samples(&signature), 0);
        assert_eq!(
            recorder.history.lock().unwrap().failure_samples(&signature),
            1
        );
        assert!(recorder.estimate(&signature, None).is_none());
    }

    #[test]
    fn cancelled_target_never_feeds_success_percentile() {
        let (recorder, _temp) = recorder();
        let signature = sig(1);
        recorder.observe(&started(1, Some("build"), Some(1)));
        recorder.observe(&cancelled(1, None));
        assert_eq!(recorder.success_samples(&signature), 0);
        let (cancelled, superseded, timed_out) =
            recorder.history.lock().unwrap().excluded_counts(&signature);
        assert_eq!((cancelled, superseded, timed_out), (1, 0, 0));
    }

    #[test]
    fn superseded_target_counts_excluded_never_success() {
        let (recorder, _temp) = recorder();
        let signature = sig(1);
        recorder.observe(&started(1, Some("build"), Some(1)));
        recorder.observe(&cancelled(1, Some(2)));
        assert_eq!(recorder.success_samples(&signature), 0);
        let (cancelled, superseded, timed_out) =
            recorder.history.lock().unwrap().excluded_counts(&signature);
        assert_eq!((cancelled, superseded, timed_out), (0, 1, 0));

        // Finished with superseded_by is likewise never success.
        recorder.observe(&started(3, Some("build"), Some(1)));
        recorder.observe(&finished(3, false, Some(4)));
        assert_eq!(recorder.success_samples(&signature), 0);
        let (_, superseded, _) = recorder.history.lock().unwrap().excluded_counts(&signature);
        assert_eq!(superseded, 2);
    }

    #[test]
    fn timeout_classification_excludes_and_removes_association() {
        let (recorder, _temp) = recorder();
        let signature = sig(1);
        recorder.observe(&started(1, Some("build"), Some(1)));
        recorder.note_timeout(1);
        assert_eq!(recorder.in_flight(), 0);
        let (_, _, timed_out) = recorder.history.lock().unwrap().excluded_counts(&signature);
        assert_eq!(timed_out, 1);
        // The later real terminal event is a no-op: never success.
        recorder.observe(&finished(1, false, None));
        assert_eq!(recorder.success_samples(&signature), 0);
    }

    #[test]
    fn duplicate_terminal_event_is_a_noop() {
        let (recorder, _temp) = recorder();
        let signature = sig(1);
        recorder.observe(&started(1, Some("build"), Some(1)));
        recorder.observe(&finished(1, false, None));
        recorder.observe(&finished(1, false, None));
        recorder.observe(&cancelled(1, None));
        assert_eq!(recorder.success_samples(&signature), 1);
        let (cancelled, _, _) = recorder.history.lock().unwrap().excluded_counts(&signature);
        assert_eq!(cancelled, 0, "duplicate terminal must not double-record");
    }

    #[test]
    fn estimate_at_start_is_frozen_and_survives_terminal() {
        let (recorder, _temp) = recorder();
        let signature = sig(1);
        // Two prior samples -> a measured estimate exists at run start.
        recorder.observe(&started(1, Some("build"), Some(1)));
        recorder.observe(&finished(1, false, None));
        recorder.observe(&started(2, Some("build"), Some(1)));
        recorder.observe(&finished(2, false, None));

        // Generation 3 starts: estimate frozen from 2 samples (typical 40k).
        recorder.observe(&started(3, Some("build"), Some(1)));
        let at_start = recorder.estimate_at_start(3).expect("captured");
        assert_eq!(at_start.typical_ms, 40_000);
        assert_eq!(at_start.samples, 2);

        // The run finishes and adds a third sample; the captured estimate for
        // generation 3 must NOT change (contract §6: snapshot-at-run-start).
        recorder.observe(&finished(3, false, None));
        let after = recorder
            .estimate_at_start(3)
            .expect("retained past terminal");
        assert_eq!(after, at_start);
        assert_eq!(after.samples, 2, "estimate never re-derived mid-run");
        // New runs see the updated history.
        recorder.observe(&started(4, Some("build"), Some(1)));
        let newer = recorder.estimate_at_start(4).expect("captured");
        assert_eq!(newer.samples, 3);
    }

    #[test]
    fn non_target_runs_are_ignored() {
        let (recorder, _temp) = recorder();
        recorder.observe(&started(1, None, None));
        recorder.observe(&finished(1, false, None));
        assert_eq!(recorder.in_flight(), 0);
        assert_eq!(recorder.history.lock().unwrap().success_samples(&sig(1)), 0);
        // Also: no association is created for a signature-less start.
        recorder.observe(&started(2, Some("build"), None));
        recorder.observe(&finished(2, false, None));
        assert_eq!(recorder.history.lock().unwrap().success_samples(&sig(1)), 0);
    }

    #[test]
    fn restart_loads_persisted_history() {
        let (recorder, temp) = recorder();
        let signature = sig(1);
        recorder.observe(&started(1, Some("build"), Some(1)));
        recorder.observe(&finished(1, false, None));
        assert_eq!(recorder.success_samples(&signature), 1);

        // New recorder over the same store: history survives "restart".
        let store = DurationStore::new(temp.0.join("run-durations-v1.json"));
        let restarted = DurationRecorder::new(store);
        assert_eq!(restarted.success_samples(&signature), 1);
        assert_eq!(
            restarted.estimate(&signature, None).unwrap().typical_ms,
            40_000
        );
    }

    #[test]
    fn local_and_control_runs_share_the_same_recording_path() {
        // Both schedule paths attach target + signature structurally; the
        // recorder treats them identically regardless of trigger string.
        let (recorder, _temp) = recorder();
        let signature = sig(1);
        recorder.observe(&started(1, Some("build"), Some(1))); // local fzz run
        recorder.observe(&finished(1, false, None));
        recorder.observe(&started(2, Some("build"), Some(1))); // control run
        recorder.observe(&finished(2, false, None));
        assert_eq!(recorder.success_samples(&signature), 2);
    }

    #[test]
    fn in_flight_associations_stay_bounded() {
        let (recorder, _temp) = recorder();
        // Saturate the bound without terminal events; the recorder must
        // evict oldest and never grow without limit.
        for run_id in 1..=MAX_ASSOCIATIONS as u64 + 10 {
            recorder.observe(&started(run_id, Some("build"), Some(1)));
        }
        assert!(recorder.in_flight() <= MAX_ASSOCIATIONS);
    }
}
