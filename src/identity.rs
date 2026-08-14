//! Typed monotonic identities (contract §1).
//!
//! Correlation fields are typed identities with defined lifetimes: event
//! batch, generation, task, and group occurrence. No identity is derived from
//! timestamps, command strings, or vector positions. All IDs are unique
//! within one watcher instance; restart changes the instance and IDs from
//! different instances are never compared.

use serde_derive::Serialize;
use std::sync::atomic::{AtomicU64, Ordering};

/// Event batch identity: the maximal set of filesystem events coalesced by
/// debounce into one trigger. Monotonic within one watcher instance; the
/// complete normalized changed-path set rides along in [`Batch`].
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, PartialOrd, Ord, Serialize)]
pub struct BatchId(pub u64);

/// Generation identity: one scheduled run-plan execution. Monotonic within
/// one watcher instance and never reused after terminal outcome.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, PartialOrd, Ord, Serialize)]
pub struct GenerationId(pub u64);

/// One normalized event batch: identity plus the complete changed-path set
/// (deduplicated and deterministically ordered).
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Batch {
    pub id: BatchId,
    pub changed: Vec<String>,
}

impl Batch {
    /// Normalizes raw event paths into the deterministic changed-path set:
    /// deduplicated and lexicographically sorted. Empty input yields an empty
    /// batch (no scheduling decision).
    pub fn normalized(id: BatchId, mut paths: Vec<String>) -> Self {
        paths.sort();
        paths.dedup();
        Self { id, changed: paths }
    }

    pub fn is_empty(&self) -> bool {
        self.changed.is_empty()
    }
}

/// Atomic monotonic sequence for instance-scoped IDs. Never reuses a value
/// within the owning watcher instance; a fresh sequence starts at 1.
#[derive(Default)]
pub struct AtomicSequence {
    next: AtomicU64,
}

impl AtomicSequence {
    pub fn new() -> Self {
        Self::default()
    }

    /// Returns the next value in strict increasing order, starting at 1.
    pub fn next(&self) -> u64 {
        self.next.fetch_add(1, Ordering::Relaxed) + 1
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn batch_and_generation_ids_serialize_as_plain_numbers() {
        let batch = serde_json::to_value(BatchId(7)).unwrap();
        let generation = serde_json::to_value(GenerationId(42)).unwrap();
        assert_eq!(batch, serde_json::json!(7));
        assert_eq!(generation, serde_json::json!(42));
    }

    #[test]
    fn atomic_sequence_is_monotonic_and_never_reuses() {
        let sequence = AtomicSequence::new();
        let ids: Vec<u64> = (0..100).map(|_| sequence.next()).collect();
        for window in ids.windows(2) {
            assert!(window[0] < window[1], "ids must be strictly increasing");
        }
        assert_eq!(ids[0], 1, "sequences start at 1");
        assert_eq!(ids.len(), 100);
        // A fresh instance restarts the sequence: cross-instance ids are
        // intentionally comparable only within one sequence.
        assert_eq!(AtomicSequence::new().next(), 1);
    }

    #[test]
    fn batch_normalization_dedupes_and_sorts() {
        let batch = Batch::normalized(
            BatchId(3),
            vec!["b.txt".to_owned(), "a.txt".to_owned(), "b.txt".to_owned()],
        );
        assert_eq!(batch.changed, vec!["a.txt".to_owned(), "b.txt".to_owned()]);
        assert_eq!(batch.id, BatchId(3));
        assert!(!batch.is_empty());
    }

    #[test]
    fn empty_batch_is_an_explicit_noop() {
        let batch = Batch::normalized(BatchId(1), vec![]);
        assert!(batch.is_empty());
        assert!(batch.changed.is_empty());
    }
}
