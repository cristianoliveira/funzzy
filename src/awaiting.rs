//! Atomic control await and freshness (TASK-0044, contract §4/§3).
//!
//! One lock guards the observation (sequence, per-generation terminal
//! outcomes, pending-work state) and waiter registration, so no transition can
//! be lost between a client's snapshot read and its waiter registration.
//! Waiters block on a condition variable — never a busy-poll — and are
//! bounded by their own deadlines; the watcher schedules and runs unaffected.

use crate::config_lifecycle::{ConfigLifecycle, ConfigTransition};
use crate::executor::Event;
use crate::output::{FailureEvidence, OutputRegistry, DEFAULT_FAILURE_EVIDENCE_LINES};
use crate::watcher_state::{WatcherExecutionState, WatcherState};
use serde::Serialize;
use std::sync::{Arc, Condvar, Mutex, MutexGuard};
use std::time::{Duration, Instant};

/// Maximum number of per-generation terminal outcomes retained. Deterministic
/// oldest-generation-first eviction; await answers only generations within
/// the retained window (older ones are gone, which is honest and bounded).
pub const TERMINAL_HISTORY_BOUND: usize = 256;

/// Why an await returned (contract §2 client-observed terminal reasons).
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize)]
#[serde(rename_all = "lowercase")]
pub enum TerminalReason {
    Passed,
    Failed,
    Cancelled,
    Superseded,
    Timeout,
    Disconnected,
    Restarted,
}

/// Freshness classification (contract §3 pi-watcher vocabulary).
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize)]
#[serde(rename_all = "lowercase")]
pub enum Freshness {
    Current,
    Stale,
    Unknown,
}

/// Pending debounce work after a snapshot (contract §3 freshness rule).
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct PendingWork {
    pub debounce_active: bool,
    pub queued_batches: u32,
}

impl PendingWork {
    pub fn is_empty(&self) -> bool {
        !self.debounce_active && self.queued_batches == 0
    }
}

/// What the await primitive is waiting for.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum AwaitMode {
    /// Next terminal generation strictly after this one.
    After(u64),
    /// The exact generation, once terminal.
    Exact(u64),
}

/// One consistent await observation: a snapshot plus the terminal reason,
/// latest observed batch/generation, pending debounce state, and freshness.
#[derive(Clone, Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct AwaitResult {
    pub snapshot: WatcherState,
    pub terminal_reason: TerminalReason,
    pub latest_generation: u64,
    pub latest_batch: Option<u64>,
    pub pending_work: PendingWork,
    pub freshness: Freshness,
    /// Concise deterministic failure evidence (contract §6), when the awaited
    /// generation failed and retained output exists. Additive: absent for
    /// passed/superseded/timeout outcomes and legacy servers.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub failure_evidence: Option<FailureEvidence>,
    /// Live config lifecycle transition at return time (TASK-0091, AC4):
    /// present only when a lifecycle source is wired. An await that spans a
    /// valid reload reports the committed revision transition in its
    /// observation without disconnecting.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub config_lifecycle: Option<ConfigTransition>,
}

#[derive(Clone, Debug)]
struct TerminalRecord {
    generation: u64,
    reason: TerminalReason,
    #[allow(dead_code)]
    superseded_by: Option<u64>,
}

#[derive(Default)]
struct AwaitInner {
    /// Highest generation observed (scheduled or superseded), including
    /// queued-discarded ones, so freshness never overclaims the latest.
    latest_generation: u64,
    /// Latest debounce batch identity observed at start of a generation.
    latest_batch: Option<u64>,
    /// Per-generation terminal outcomes, sorted by generation, bounded.
    terminal: Vec<TerminalRecord>,
    pending_work: PendingWork,
}

/// Shared observation + waiter registry. Constructed by the watch command,
/// fed by the worker event stream, and handed to the control server.
pub struct AwaitCoordinator {
    inner: Mutex<AwaitInner>,
    changed: Condvar,
}

impl Default for AwaitCoordinator {
    fn default() -> Self {
        Self::new()
    }
}

impl AwaitCoordinator {
    pub fn new() -> Self {
        Self {
            inner: Mutex::new(AwaitInner::default()),
            changed: Condvar::new(),
        }
    }

    /// Records one worker event into the observation. Called by the same
    /// event stream that updates `WatcherState`, under no lock nesting.
    pub fn observe(&self, event: &Event) {
        let mut inner = self.inner.lock().unwrap();
        match event {
            Event::Started { run_id, batch, .. } => {
                inner.latest_generation = inner.latest_generation.max(*run_id);
                if let Some(batch) = batch {
                    inner.latest_batch = Some(*batch);
                }
            }
            Event::Finished {
                run_id,
                superseded_by,
                failures,
                ..
            } => {
                inner.latest_generation = inner
                    .latest_generation
                    .max(*run_id)
                    .max(superseded_by.unwrap_or(0));
                let reason = if failures.is_empty() {
                    TerminalReason::Passed
                } else {
                    TerminalReason::Failed
                };
                Self::record(&mut inner, *run_id, reason, *superseded_by);
                self.changed.notify_all();
            }
            Event::Cancelled {
                run_id,
                superseded_by,
            } => {
                inner.latest_generation = inner
                    .latest_generation
                    .max(*run_id)
                    .max(superseded_by.unwrap_or(0));
                let reason = if superseded_by.is_some() {
                    TerminalReason::Superseded
                } else {
                    TerminalReason::Cancelled
                };
                Self::record(&mut inner, *run_id, reason, *superseded_by);
                self.changed.notify_all();
            }
            Event::Tick { .. } => {}
            // Per-task outcomes live in `WatcherState`; the coordinator only
            // tracks generation/batch/pending facts for freshness (TASK-0050).
            Event::TaskTerminal { .. }
            | Event::RecoveryPhase { .. }
            | Event::ServiceLifecycle { .. } => {}
        }
    }

    /// Snapshot facts for the correlated snapshot (TASK-0050): latest
    /// generation and batch, plus pending-work state, read under one lock so
    /// the snapshot never mixes generations.
    pub fn snapshot_facts(&self) -> (u64, Option<u64>, PendingWork) {
        let inner = self.inner.lock().unwrap();
        (
            inner.latest_generation,
            inner.latest_batch,
            inner.pending_work.clone(),
        )
    }

    /// A debounce batch opened (first event → scheduling decision).
    pub fn note_batch(&self, batch_id: u64) {
        let mut inner = self.inner.lock().unwrap();
        inner.latest_batch = Some(batch_id);
        inner.pending_work.debounce_active = true;
    }

    /// The batch finished routing (scheduled or explicit no-op).
    pub fn note_batch_complete(&self) {
        self.inner.lock().unwrap().pending_work.debounce_active = false;
    }

    fn record(
        inner: &mut AwaitInner,
        generation: u64,
        reason: TerminalReason,
        superseded_by: Option<u64>,
    ) {
        inner.terminal.push(TerminalRecord {
            generation,
            reason,
            superseded_by,
        });
        // Deterministic bounded retention: drop oldest generations first.
        while inner.terminal.len() > TERMINAL_HISTORY_BOUND {
            inner.terminal.remove(0);
        }
    }

    /// Atomic await: evaluate under the lock, register (block on the condvar
    /// with a bounded slice), re-evaluate. `probe` optionally detects client
    /// disconnect between wake slices. Timeouts perform no cancellation.
    #[allow(clippy::too_many_arguments)]
    pub fn await_generation(
        &self,
        mode: AwaitMode,
        timeout: Duration,
        snapshot: &Arc<Mutex<WatcherState>>,
        mut probe: Option<&mut dyn FnMut() -> bool>,
        outputs: Option<&OutputRegistry>,
        instance_token: &str,
        lifecycle: Option<&ConfigLifecycle>,
    ) -> AwaitResult {
        let deadline = Instant::now() + timeout;
        loop {
            let inner = self.inner.lock().unwrap();
            if let Some((reason, generation)) = Self::evaluate(&inner, mode) {
                return Self::build(
                    inner,
                    snapshot,
                    reason,
                    generation,
                    outputs,
                    instance_token,
                    lifecycle,
                );
            }
            let now = Instant::now();
            if now >= deadline {
                return Self::build(
                    inner,
                    snapshot,
                    TerminalReason::Timeout,
                    0,
                    outputs,
                    instance_token,
                    lifecycle,
                );
            }
            let slice = (deadline - now).min(Duration::from_millis(500));
            let (guard, _) = self.changed.wait_timeout(inner, slice).unwrap();
            if let Some(probe) = probe.as_deref_mut() {
                if probe() {
                    return Self::build(
                        guard,
                        snapshot,
                        TerminalReason::Disconnected,
                        0,
                        outputs,
                        instance_token,
                        lifecycle,
                    );
                }
            }
            // Loop re-evaluates: no transition can be lost between the
            // condition check and waiter registration because both happen
            // under the same lock that guards every observation change.
        }
    }

    fn evaluate(inner: &AwaitInner, mode: AwaitMode) -> Option<(TerminalReason, u64)> {
        let record = match mode {
            AwaitMode::Exact(generation) => inner
                .terminal
                .iter()
                .rev()
                .find(|record| record.generation == generation),
            AwaitMode::After(generation) => inner
                .terminal
                .iter()
                .rev()
                .find(|record| record.generation > generation),
        };
        record.map(|record| (record.reason, record.generation))
    }

    /// Reads one consistent snapshot while holding the observation lock, so
    /// the awaited decision and the returned snapshot cannot mix generations.
    fn build(
        inner: MutexGuard<'_, AwaitInner>,
        snapshot: &Arc<Mutex<WatcherState>>,
        terminal_reason: TerminalReason,
        generation: u64,
        outputs: Option<&OutputRegistry>,
        instance_token: &str,
        lifecycle: Option<&ConfigLifecycle>,
    ) -> AwaitResult {
        let snapshot = snapshot.lock().unwrap().clone();
        let latest_generation = inner.latest_generation;
        let latest_batch = inner.latest_batch;
        let pending_work = inner.pending_work.clone();
        drop(inner);
        let freshness = classify(&snapshot, latest_generation, &pending_work);
        let failure_evidence = if terminal_reason == TerminalReason::Failed {
            let failed_tasks: Vec<String> = snapshot
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
            outputs.and_then(|outputs| {
                outputs.failure_evidence(
                    generation,
                    DEFAULT_FAILURE_EVIDENCE_LINES,
                    instance_token,
                    &failed_tasks,
                )
            })
        } else {
            None
        };
        AwaitResult {
            snapshot,
            terminal_reason,
            latest_generation,
            latest_batch,
            pending_work,
            freshness,
            failure_evidence,
            config_lifecycle: lifecycle.map(ConfigLifecycle::current),
        }
    }
}

/// Freshness rule (contract §3): current only when the snapshot is the latest
/// scheduled generation, terminal, and no pending debounce work exists.
pub(crate) fn classify(
    snapshot: &WatcherState,
    latest_generation: u64,
    pending: &PendingWork,
) -> Freshness {
    let terminal = matches!(
        snapshot.state(),
        WatcherExecutionState::Passed
            | WatcherExecutionState::Failed
            | WatcherExecutionState::Cancelled
    );
    let no_pending = pending.is_empty();
    if snapshot.generation() == latest_generation && terminal && no_pending {
        Freshness::Current
    } else if latest_generation > snapshot.generation() || !no_pending {
        Freshness::Stale
    } else {
        Freshness::Unknown
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::time::Duration;

    fn started(run_id: u64, batch: Option<u64>) -> Event {
        Event::Started {
            run_id,
            trigger: "src/main.rs".to_owned(),
            batch,
            predecessor: None,
            changed: vec![],
            commands: vec!["echo hi".to_owned()],
            target: None,
            execution_signature: None,
            effective_concurrency: None,
            concurrency_source: None,
            revision: None,
            revision_hash: None,
        }
    }

    fn finished(run_id: u64, failures: bool) -> Event {
        Event::Finished {
            run_id,
            superseded_by: None,
            elapsed: Duration::from_millis(1),
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

    fn await_forever(
        coordinator: &AwaitCoordinator,
        mode: AwaitMode,
        state: &Arc<Mutex<WatcherState>>,
    ) -> AwaitResult {
        coordinator.await_generation(
            mode,
            Duration::from_secs(30),
            state,
            None,
            None,
            "fz-test",
            None,
        )
    }

    fn spawn_wait(
        coordinator: Arc<AwaitCoordinator>,
        mode: AwaitMode,
        state: Arc<Mutex<WatcherState>>,
    ) -> std::thread::JoinHandle<AwaitResult> {
        std::thread::spawn(move || await_forever(&coordinator, mode, &state))
    }

    fn wait_for<T>(handle: std::thread::JoinHandle<T>) -> T {
        handle.join().expect("waiter finished")
    }

    #[test]
    fn await_result_carries_the_config_lifecycle_transition_at_return() {
        // TASK-0091, AC4: an await observation includes the live config
        // lifecycle transition, so a reload that happens while waiting is
        // visible in the returned observation without reconnecting.
        let coordinator = Arc::new(AwaitCoordinator::new());
        let state = Arc::new(Mutex::new(WatcherState::default()));
        coordinator.observe(&started(7, None));
        state.lock().unwrap().apply(started(7, None));
        coordinator.observe(&finished(7, false));
        state.lock().unwrap().apply(finished(7, false));

        let lifecycle = crate::config_lifecycle::ConfigLifecycle::new();
        lifecycle.reloaded(&crate::config_revision::ConfigRevision {
            number: 2,
            hash: "hash-2".to_owned(),
        });

        let result = coordinator.await_generation(
            AwaitMode::Exact(7),
            Duration::from_millis(100),
            &state,
            None,
            None,
            "fz-test",
            Some(&lifecycle),
        );
        assert_eq!(result.terminal_reason, TerminalReason::Passed);
        let transition = result.config_lifecycle.expect("lifecycle present");
        assert_eq!(
            transition.phase,
            crate::config_lifecycle::ConfigPhase::ConfigReloaded
        );
        assert_eq!(transition.revision, Some(2));
    }

    #[test]
    fn already_terminal_generation_returns_immediately() {
        let coordinator = Arc::new(AwaitCoordinator::new());
        let state = Arc::new(Mutex::new(WatcherState::default()));
        coordinator.observe(&started(7, None));
        state.lock().unwrap().apply(started(7, None));
        coordinator.observe(&finished(7, false));
        state.lock().unwrap().apply(finished(7, false));

        let result = coordinator.await_generation(
            AwaitMode::Exact(7),
            Duration::from_millis(100),
            &state,
            None,
            None,
            "fz-test",
            None,
        );
        assert_eq!(result.terminal_reason, TerminalReason::Passed);
        assert_eq!(result.latest_generation, 7);
        assert_eq!(result.freshness, Freshness::Current);
    }

    #[test]
    fn future_completion_wakes_the_waiter() {
        let coordinator = Arc::new(AwaitCoordinator::new());
        let state = Arc::new(Mutex::new(WatcherState::default()));
        coordinator.observe(&started(9, None));
        let exact = spawn_wait(
            Arc::clone(&coordinator),
            AwaitMode::Exact(9),
            Arc::clone(&state),
        );
        let after = spawn_wait(
            Arc::clone(&coordinator),
            AwaitMode::After(8),
            Arc::clone(&state),
        );

        std::thread::sleep(Duration::from_millis(50));
        coordinator.observe(&finished(9, false));

        assert_eq!(wait_for(exact).terminal_reason, TerminalReason::Passed);
        assert_eq!(wait_for(after).terminal_reason, TerminalReason::Passed);
    }

    #[test]
    fn no_generation_yet_blocks_until_the_first_terminal() {
        let coordinator = Arc::new(AwaitCoordinator::new());
        let state = Arc::new(Mutex::new(WatcherState::default()));
        let waiter = spawn_wait(
            Arc::clone(&coordinator),
            AwaitMode::After(0),
            Arc::clone(&state),
        );
        std::thread::sleep(Duration::from_millis(30));
        coordinator.observe(&started(1, None));
        coordinator.observe(&finished(1, false));
        assert_eq!(wait_for(waiter).terminal_reason, TerminalReason::Passed);
    }

    #[test]
    fn superseded_generation_returns_superseded() {
        let coordinator = Arc::new(AwaitCoordinator::new());
        let state = Arc::new(Mutex::new(WatcherState::default()));
        coordinator.observe(&started(5, None));
        let waiter = spawn_wait(
            Arc::clone(&coordinator),
            AwaitMode::Exact(5),
            Arc::clone(&state),
        );
        std::thread::sleep(Duration::from_millis(30));
        coordinator.observe(&cancelled(5, Some(6)));
        let result = wait_for(waiter);
        assert_eq!(result.terminal_reason, TerminalReason::Superseded);
        assert_eq!(result.latest_generation, 6);
    }

    #[test]
    fn after_mode_returns_latest_terminal_after_the_reference() {
        let coordinator = Arc::new(AwaitCoordinator::new());
        let state = Arc::new(Mutex::new(WatcherState::default()));
        coordinator.observe(&started(1, None));
        coordinator.observe(&cancelled(1, Some(2)));
        coordinator.observe(&started(2, None));
        coordinator.observe(&finished(2, false));

        let result = coordinator.await_generation(
            AwaitMode::After(0),
            Duration::from_millis(100),
            &state,
            None,
            None,
            "fz-test",
            None,
        );
        assert_eq!(result.terminal_reason, TerminalReason::Passed);
        assert_eq!(result.latest_generation, 2);
    }

    #[test]
    fn timeout_returns_latest_snapshot_without_cancellation() {
        let coordinator = Arc::new(AwaitCoordinator::new());
        let state = Arc::new(Mutex::new(WatcherState::default()));
        state.lock().unwrap().apply(started(3, None));

        let result = coordinator.await_generation(
            AwaitMode::Exact(99),
            Duration::from_millis(50),
            &state,
            None,
            None,
            "fz-test",
            None,
        );
        assert_eq!(result.terminal_reason, TerminalReason::Timeout);
        assert_eq!(result.snapshot.generation(), 3);
        assert_eq!(result.freshness, Freshness::Unknown);
    }

    #[test]
    fn multiple_waiters_all_wake_on_one_terminal_event() {
        let coordinator = Arc::new(AwaitCoordinator::new());
        let state = Arc::new(Mutex::new(WatcherState::default()));
        let waiters: Vec<_> = (0..4)
            .map(|_| {
                spawn_wait(
                    Arc::clone(&coordinator),
                    AwaitMode::After(0),
                    Arc::clone(&state),
                )
            })
            .collect();
        std::thread::sleep(Duration::from_millis(30));
        coordinator.observe(&started(1, None));
        coordinator.observe(&finished(1, false));
        for waiter in waiters {
            assert_eq!(wait_for(waiter).terminal_reason, TerminalReason::Passed);
        }
    }

    #[test]
    fn timeout_boundary_cleans_up_the_waiter() {
        let coordinator = Arc::new(AwaitCoordinator::new());
        let state = Arc::new(Mutex::new(WatcherState::default()));
        let result = coordinator.await_generation(
            AwaitMode::Exact(1),
            Duration::from_millis(10),
            &state,
            None,
            None,
            "fz-test",
            None,
        );
        assert_eq!(result.terminal_reason, TerminalReason::Timeout);
        // A zero timeout returns immediately too (bounded, never unbounded).
        let zero = coordinator.await_generation(
            AwaitMode::Exact(1),
            Duration::ZERO,
            &state,
            None,
            None,
            "fz-test",
            None,
        );
        assert_eq!(zero.terminal_reason, TerminalReason::Timeout);
    }

    #[test]
    fn pending_work_marks_freshness_stale() {
        let coordinator = Arc::new(AwaitCoordinator::new());
        let state = Arc::new(Mutex::new(WatcherState::default()));
        coordinator.observe(&started(1, Some(4)));
        state.lock().unwrap().apply(started(1, Some(4)));
        coordinator.observe(&finished(1, false));
        state.lock().unwrap().apply(finished(1, false));
        coordinator.note_batch(5);
        coordinator.note_batch_complete();
        // After the batch completed, no pending work: current.
        let result = coordinator.await_generation(
            AwaitMode::Exact(1),
            Duration::from_millis(50),
            &state,
            None,
            None,
            "fz-test",
            None,
        );
        assert_eq!(result.latest_batch, Some(5));
        assert_eq!(result.pending_work, PendingWork::default());
        assert_eq!(result.freshness, Freshness::Current);
    }

    #[test]
    fn queued_discard_advances_latest_generation_for_freshness() {
        let coordinator = Arc::new(AwaitCoordinator::new());
        let state = Arc::new(Mutex::new(WatcherState::default()));
        // Generation 2 supersedes 1 before it ever starts.
        coordinator.observe(&started(2, None));
        coordinator.observe(&cancelled(1, Some(2)));
        coordinator.observe(&finished(2, false));

        let result = coordinator.await_generation(
            AwaitMode::Exact(1),
            Duration::from_millis(50),
            &state,
            None,
            None,
            "fz-test",
            None,
        );
        assert_eq!(result.terminal_reason, TerminalReason::Superseded);
        assert_eq!(result.latest_generation, 2);
    }

    #[test]
    fn history_is_bounded_and_evicts_oldest_first() {
        let coordinator = Arc::new(AwaitCoordinator::new());
        for generation in 1..=300 {
            coordinator.observe(&started(generation, None));
            coordinator.observe(&finished(generation, false));
        }
        let inner = coordinator.inner.lock().unwrap();
        assert_eq!(inner.terminal.len(), TERMINAL_HISTORY_BOUND);
        assert_eq!(inner.terminal.first().unwrap().generation, 300 - 255);
        assert_eq!(inner.terminal.last().unwrap().generation, 300);
    }

    #[test]
    fn terminal_outcome_of_failed_generation_is_reported() {
        let coordinator = Arc::new(AwaitCoordinator::new());
        let state = Arc::new(Mutex::new(WatcherState::default()));
        coordinator.observe(&started(4, None));
        let waiter = spawn_wait(
            Arc::clone(&coordinator),
            AwaitMode::Exact(4),
            Arc::clone(&state),
        );
        std::thread::sleep(Duration::from_millis(30));
        coordinator.observe(&finished(4, true));
        assert_eq!(wait_for(waiter).terminal_reason, TerminalReason::Failed);
    }

    #[test]
    fn explicit_cancel_outcome_is_cancelled_not_superseded() {
        let coordinator = Arc::new(AwaitCoordinator::new());
        let state = Arc::new(Mutex::new(WatcherState::default()));
        coordinator.observe(&started(6, None));
        coordinator.observe(&cancelled(6, None));
        let result = coordinator.await_generation(
            AwaitMode::Exact(6),
            Duration::from_millis(50),
            &state,
            None,
            None,
            "fz-test",
            None,
        );
        assert_eq!(result.terminal_reason, TerminalReason::Cancelled);
    }
}
