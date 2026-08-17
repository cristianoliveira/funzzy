//! Pure bounded duration history and deterministic estimator (TASK-0052).
//!
//! No filesystem, clock, control-socket, executor, or Pi concerns: the
//! module only records completed durations keyed by a stable execution
//! signature and derives deterministic estimates from them. Persistence
//! (TASK-0053), executor recording (TASK-0054), and protocol exposure
//! (TASK-0055) live elsewhere; this module is the pure core they build on.
//!
//! Contract: `docs/RUN-DURATION-ESTIMATES-CONTRACT.md` §1–§3, §8.

use crate::plan::ExecutionSignature;
use serde::Serialize;
use std::collections::{BTreeMap, VecDeque};

/// Success samples retained per signature, oldest-first eviction (§2).
pub const SUCCESS_RETENTION: usize = 20;
/// Floor below which no recommendation is ever produced (§2).
pub const DEFAULT_FLOOR_MS: u64 = 10_000;
/// Safety margin: `upper * 1.5` via `upper + upper/2`, plus this addend.
pub const SAFETY_MARGIN_ADDEND_MS: u64 = 2_000;
/// Absolute cap for any recommendation (§2).
pub const ABSOLUTE_CAP_MS: u64 = 15 * 60_000;

/// Deterministic confidence band derived from sample count (§2).
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize)]
#[serde(rename_all = "lowercase")]
pub enum EstimateConfidence {
    None,
    Low,
    Medium,
    High,
}

/// Where an estimate came from (§2). This revision only emits `Measured`;
/// `Configured` is reserved for zero-history serialization (TASK-0055).
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize)]
#[serde(rename_all = "lowercase")]
pub enum EstimateSource {
    Measured,
    Configured,
}

/// Deterministic estimate for one execution signature (§2). Wire shape
/// (contract §6): camelCase fields, absent optional fields omitted.
#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct RunEstimate {
    pub typical_ms: u64,
    pub upper_ms: u64,
    pub recommended_timeout_ms: u64,
    pub samples: usize,
    pub confidence: EstimateConfidence,
    pub source: EstimateSource,
}

/// Outcome classes that must never lower a success estimate (§1).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ExcludedKind {
    Cancelled,
    Superseded,
    TimedOut,
}

/// Crate-internal persistence snapshot of one profile (TASK-0053). Serde and
/// filesystem stay in `duration_store`; this is a plain serde-free view so
/// the estimator remains independent of serialization concerns.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct ProfileSnapshot {
    pub signature: ExecutionSignature,
    pub successes: Vec<u64>,
    pub failures: Vec<u64>,
    pub cancelled: usize,
    pub superseded: usize,
    pub timed_out: usize,
}

/// Bounded per-signature outcome samples and counts (§1, §3).
#[derive(Clone, Debug, Default, PartialEq, Eq)]
struct Profile {
    /// Passed (not superseded) durations, newest last, oldest-first eviction.
    successes: VecDeque<u64>,
    /// Failed durations, separate diagnostics only (never a success baseline).
    failures: VecDeque<u64>,
    /// Cancelled/superseded/timed-out counts, never fed to percentiles.
    cancelled: usize,
    superseded: usize,
    timed_out: usize,
}

/// Pure, bounded, deterministic duration history keyed by execution
/// signature (contract §1–§3).
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct DurationHistory {
    profiles: BTreeMap<ExecutionSignature, Profile>,
}

impl DurationHistory {
    pub fn new() -> Self {
        Self::default()
    }

    /// Records a passed (not superseded) run duration as a success sample.
    /// Evicts the oldest sample when retention is exceeded.
    pub fn record_success(&mut self, signature: &ExecutionSignature, duration_ms: u64) {
        let profile = self.profiles.entry(signature.clone()).or_default();
        profile.successes.push_back(duration_ms);
        if profile.successes.len() > SUCCESS_RETENTION {
            profile.successes.pop_front();
        }
    }

    /// Records a failed run duration in the separate diagnostics bucket.
    pub fn record_failure(&mut self, signature: &ExecutionSignature, duration_ms: u64) {
        let profile = self.profiles.entry(signature.clone()).or_default();
        profile.failures.push_back(duration_ms);
        if profile.failures.len() > SUCCESS_RETENTION {
            profile.failures.pop_front();
        }
    }

    /// Counts an outcome that is excluded from the success distribution.
    pub fn record_excluded(&mut self, signature: &ExecutionSignature, kind: ExcludedKind) {
        let profile = self.profiles.entry(signature.clone()).or_default();
        match kind {
            ExcludedKind::Cancelled => profile.cancelled += 1,
            ExcludedKind::Superseded => profile.superseded += 1,
            ExcludedKind::TimedOut => profile.timed_out += 1,
        }
    }

    /// Number of retained success samples for a signature.
    pub fn success_samples(&self, signature: &ExecutionSignature) -> usize {
        self.profiles
            .get(signature)
            .map(|profile| profile.successes.len())
            .unwrap_or(0)
    }

    /// Number of retained failure samples for a signature (diagnostics).
    pub fn failure_samples(&self, signature: &ExecutionSignature) -> usize {
        self.profiles
            .get(signature)
            .map(|profile| profile.failures.len())
            .unwrap_or(0)
    }

    /// Retained excluded-outcome counts for a signature (diagnostics).
    pub fn excluded_counts(&self, signature: &ExecutionSignature) -> (usize, usize, usize) {
        self.profiles
            .get(signature)
            .map(|profile| (profile.cancelled, profile.superseded, profile.timed_out))
            .unwrap_or((0, 0, 0))
    }

    /// Crate-internal snapshot for persistence (TASK-0053): one plain view per
    /// signature, insertion-order preserved. Serde-free by design.
    pub(crate) fn snapshot(&self) -> Vec<ProfileSnapshot> {
        self.profiles
            .iter()
            .map(|(signature, profile)| ProfileSnapshot {
                signature: signature.clone(),
                successes: profile.successes.iter().copied().collect(),
                failures: profile.failures.iter().copied().collect(),
                cancelled: profile.cancelled,
                superseded: profile.superseded,
                timed_out: profile.timed_out,
            })
            .collect()
    }

    /// Rebuilds history from a persisted snapshot, enforcing the same bounds
    /// the live path enforces: sample retention is capped, oversized inputs
    /// are rejected rather than silently truncated. Crate-internal for the
    /// store adapter (TASK-0053).
    pub(crate) fn from_snapshot(snapshots: Vec<ProfileSnapshot>) -> Result<Self, String> {
        let mut history = DurationHistory::new();
        for snapshot in snapshots {
            let profile = history
                .profiles
                .entry(snapshot.signature.clone())
                .or_default();
            if snapshot.successes.len() > SUCCESS_RETENTION
                || snapshot.failures.len() > SUCCESS_RETENTION
            {
                return Err(format!(
                    "profile '{}' exceeds retention bound ({} samples)",
                    snapshot.signature.0, SUCCESS_RETENTION
                ));
            }
            profile.successes.extend(snapshot.successes);
            profile.failures.extend(snapshot.failures);
            profile.cancelled = snapshot.cancelled;
            profile.superseded = snapshot.superseded;
            profile.timed_out = snapshot.timed_out;
        }
        Ok(history)
    }

    /// Derives the deterministic estimate for a signature, or `None` when no
    /// success samples exist. `configured_floor_ms` (a `timeout_hint`, §4)
    /// raises the recommendation but never shrinks below the safety floor.
    pub fn estimate(
        &self,
        signature: &ExecutionSignature,
        configured_floor_ms: Option<u64>,
    ) -> Option<RunEstimate> {
        let successes = self.profiles.get(signature)?.successes.clone();
        if successes.is_empty() {
            return None;
        }
        let mut sorted: Vec<u64> = successes.into_iter().collect();
        sorted.sort_unstable();
        let samples = sorted.len();
        let typical_ms = median(&sorted);
        let upper_ms = nearest_rank_p90(&sorted);
        let recommended_timeout_ms = recommend_timeout(upper_ms, configured_floor_ms);
        Some(RunEstimate {
            typical_ms,
            upper_ms,
            recommended_timeout_ms,
            samples,
            confidence: confidence(samples),
            source: EstimateSource::Measured,
        })
    }
}

/// Median: middle value for odd counts, mean of the two middle values for
/// even counts. Uses u128 internally so the mean cannot overflow.
fn median(sorted: &[u64]) -> u64 {
    let n = sorted.len();
    if n == 0 {
        return 0;
    }
    if n % 2 == 1 {
        sorted[n / 2]
    } else {
        let a = sorted[n / 2 - 1] as u128;
        let b = sorted[n / 2] as u128;
        let mean = (a + b) / 2;
        mean.min(u64::MAX as u128) as u64
    }
}

/// Nearest-rank p90: the value at 1-based rank `ceil(0.9 * n)`.
fn nearest_rank_p90(sorted: &[u64]) -> u64 {
    let n = sorted.len();
    if n == 0 {
        return 0;
    }
    // ceil(0.9 * n) as 1-based rank; n*9/10 rounds down, so add n*9 + 9 over 10.
    let rank = (n * 9 + 9) / 10;
    sorted[rank - 1]
}

/// `clamp(max(configured_floor, 10s, upper*1.5 + 2s), 15m)` with saturating
/// arithmetic so overflow falls under the cap (§2, §8 row 8).
fn recommend_timeout(upper_ms: u64, configured_floor_ms: Option<u64>) -> u64 {
    let margin = upper_ms
        .saturating_add(upper_ms / 2)
        .saturating_add(SAFETY_MARGIN_ADDEND_MS);
    let floor = configured_floor_ms.unwrap_or(0).max(DEFAULT_FLOOR_MS);
    margin.max(floor).min(ABSOLUTE_CAP_MS)
}

/// Confidence bands: none 0, low 1–2, medium 3–9, high 10+ (§2).
fn confidence(samples: usize) -> EstimateConfidence {
    match samples {
        0 => EstimateConfidence::None,
        1..=2 => EstimateConfidence::Low,
        3..=9 => EstimateConfidence::Medium,
        _ => EstimateConfidence::High,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn sig(n: u64) -> ExecutionSignature {
        ExecutionSignature(format!("sig-{n}"))
    }

    fn estimate_for(samples: &[u64], floor: Option<u64>) -> RunEstimate {
        let mut history = DurationHistory::new();
        let signature = sig(1);
        for &sample in samples {
            history.record_success(&signature, sample);
        }
        history
            .estimate(&signature, floor)
            .expect("estimate must exist with samples")
    }

    #[test]
    fn empty_history_yields_no_estimate() {
        let history = DurationHistory::new();
        assert_eq!(history.estimate(&sig(1), None), None);
        assert_eq!(history.success_samples(&sig(1)), 0);
    }

    #[test]
    fn one_sample_estimate_is_low_confidence_and_measured() {
        let estimate = estimate_for(&[40_000], None);
        assert_eq!(estimate.typical_ms, 40_000);
        assert_eq!(estimate.upper_ms, 40_000);
        // max(10s, 40k*1.5 + 2s) = 62s
        assert_eq!(estimate.recommended_timeout_ms, 62_000);
        assert_eq!(estimate.samples, 1);
        assert_eq!(estimate.confidence, EstimateConfidence::Low);
        assert_eq!(estimate.source, EstimateSource::Measured);
    }

    #[test]
    fn two_samples_median_is_mean_of_middle_values() {
        let estimate = estimate_for(&[30_000, 50_000], None);
        assert_eq!(estimate.typical_ms, 40_000);
        // p90 nearest-rank: ceil(0.9*2)=2 -> 2nd value
        assert_eq!(estimate.upper_ms, 50_000);
        // max(10s, 50k*1.5+2s = 77s)
        assert_eq!(estimate.recommended_timeout_ms, 77_000);
        assert_eq!(estimate.confidence, EstimateConfidence::Low);
    }

    #[test]
    fn odd_count_median_is_middle_value() {
        let estimate = estimate_for(&[10_000, 20_000, 30_000], None);
        assert_eq!(estimate.typical_ms, 20_000);
        // p90 nearest-rank: ceil(0.9*3)=3 -> 3rd value
        assert_eq!(estimate.upper_ms, 30_000);
        assert_eq!(estimate.recommended_timeout_ms, 47_000);
        assert_eq!(estimate.confidence, EstimateConfidence::Medium);
    }

    #[test]
    fn p90_nearest_rank_uses_ceiling_rank() {
        // 10 samples: rank ceil(9)=9 -> 9th (0-based index 8)
        let samples: Vec<u64> = (1..=10).map(|v| v * 10_000).collect();
        let estimate = estimate_for(&samples, None);
        assert_eq!(estimate.typical_ms, 55_000); // mean of 5th/6th
        assert_eq!(estimate.upper_ms, 90_000); // 9th value
        assert_eq!(estimate.confidence, EstimateConfidence::High);
    }

    #[test]
    fn retention_evicts_oldest_success_sample() {
        let mut history = DurationHistory::new();
        let signature = sig(1);
        for sample in 1..=SUCCESS_RETENTION + 1 {
            history.record_success(&signature, sample as u64);
        }
        assert_eq!(history.success_samples(&signature), SUCCESS_RETENTION);
        // Oldest (1) evicted; newest (21) present.
        let estimate = history.estimate(&signature, None).unwrap();
        assert_eq!(estimate.samples, SUCCESS_RETENTION);
        let sorted: Vec<u64> = (2..=21).collect();
        assert_eq!(estimate.typical_ms, median(&sorted));
        assert_eq!(estimate.upper_ms, nearest_rank_p90(&sorted));
    }

    #[test]
    fn outlier_does_not_dominate_median_or_p90() {
        // 20 samples: 19 near 10s, one at 100s. p90 (18th of 20) excludes the
        // outlier, so the recommendation stays bounded and robust.
        let samples: Vec<u64> = (1..=19).map(|_| 10_000).chain([100_000]).collect();
        let estimate = estimate_for(&samples, None);
        assert_eq!(estimate.typical_ms, 10_000);
        assert_eq!(estimate.upper_ms, 10_000);
        // max(10s, 10k*1.5+2s = 17s) = 17s
        assert_eq!(estimate.recommended_timeout_ms, 17_000);
        assert_eq!(estimate.confidence, EstimateConfidence::High);
    }

    #[test]
    fn overflow_saturates_and_falls_under_cap() {
        let estimate = estimate_for(&[u64::MAX], None);
        assert_eq!(estimate.typical_ms, u64::MAX);
        assert_eq!(estimate.upper_ms, u64::MAX);
        assert_eq!(estimate.recommended_timeout_ms, ABSOLUTE_CAP_MS);
    }

    #[test]
    fn configured_floor_raises_recommendation() {
        let floor = 2 * 60_000; // 2m hint
        let estimate = estimate_for(&[30_000, 50_000], Some(floor));
        // margin 77s < floor 120s -> floor wins
        assert_eq!(estimate.recommended_timeout_ms, floor);
    }

    #[test]
    fn safety_floor_wins_without_configured_hint() {
        // Margin below the 10s safety floor -> floor wins.
        let estimate = estimate_for(&[1_000], None);
        assert_eq!(estimate.recommended_timeout_ms, DEFAULT_FLOOR_MS);
    }

    #[test]
    fn absolute_cap_bounds_huge_upper() {
        let estimate = estimate_for(&[60 * 60_000], None);
        assert_eq!(estimate.recommended_timeout_ms, ABSOLUTE_CAP_MS);
    }

    #[test]
    fn confidence_boundaries_are_exact() {
        let assert_confidence = |samples: usize, expected: EstimateConfidence| {
            let samples_vec: Vec<u64> = (1..=samples as u64).collect();
            assert_eq!(
                estimate_for(&samples_vec, None).confidence,
                expected,
                "samples={samples}"
            );
        };
        assert_confidence(1, EstimateConfidence::Low);
        assert_confidence(2, EstimateConfidence::Low);
        assert_confidence(3, EstimateConfidence::Medium);
        assert_confidence(9, EstimateConfidence::Medium);
        assert_confidence(10, EstimateConfidence::High);
        assert_confidence(20, EstimateConfidence::High);
    }

    #[test]
    fn failed_and_excluded_samples_never_lower_success_estimate() {
        let mut history = DurationHistory::new();
        let signature = sig(1);
        history.record_success(&signature, 40_000);
        history.record_failure(&signature, 10_000);
        history.record_excluded(&signature, ExcludedKind::Cancelled);
        history.record_excluded(&signature, ExcludedKind::Superseded);
        history.record_excluded(&signature, ExcludedKind::TimedOut);

        let estimate = history.estimate(&signature, None).unwrap();
        assert_eq!(estimate.typical_ms, 40_000);
        assert_eq!(estimate.upper_ms, 40_000);
        assert_eq!(estimate.samples, 1);
        assert_eq!(history.failure_samples(&signature), 1);
        assert_eq!(
            history.excluded_counts(&signature),
            (1, 1, 1),
            "cancelled, superseded, timed_out"
        );
    }

    #[test]
    fn signatures_are_isolated_profiles() {
        let mut history = DurationHistory::new();
        history.record_success(&sig(1), 40_000);
        history.record_success(&sig(2), 90_000);
        assert_eq!(history.success_samples(&sig(1)), 1);
        assert_eq!(history.success_samples(&sig(2)), 1);
        assert_eq!(history.estimate(&sig(1), None).unwrap().typical_ms, 40_000);
        assert_eq!(history.estimate(&sig(2), None).unwrap().typical_ms, 90_000);
        assert_eq!(history.estimate(&sig(3), None), None);
    }

    #[test]
    fn even_mean_cannot_overflow() {
        let estimate = estimate_for(&[u64::MAX, u64::MAX], None);
        assert_eq!(estimate.typical_ms, u64::MAX);
    }
}
