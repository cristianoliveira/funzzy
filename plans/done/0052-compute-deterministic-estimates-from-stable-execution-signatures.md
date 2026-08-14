---
id: TASK-0052
title: Compute deterministic estimates from stable execution signatures
status: done
depends_on: [TASK-0051, TASK-0025, TASK-0032]
priority: high
tags: [rust, domain, statistics, signature, determinism, tdd]
---

# Compute deterministic estimates from stable execution signatures

## Problem
Raw last-run duration is noisy and becomes stale when command topology or task context changes; Funzzy needs a pure robust estimator keyed by deterministic execution identity.

## Context

Add pure duration-history domain plus stable `RunPlan` signature. Use maintained stable hash algorithm; do not use process-randomized/default hasher.

## Acceptance criteria

- [x] Tests first cover empty, one/two samples, odd/even median, nearest-rank p90, retention eviction, outlier, overflow, floor, cap, and confidence boundaries.
- [x] Last N successful durations produce deterministic `typicalMs`, `upperMs`, recommendation, samples, confidence, and source.
- [x] Failed durations are separate diagnostics; cancelled/superseded/timed-out samples cannot lower success estimate.
- [x] Signature canonicalizes stage/barrier order, task/group identity, shell versus argv boundaries, cwd, declared env, jobs, fail-fast, and schema version.
- [x] Equivalent environment map insertion order produces same signature.
- [x] Command, argv boundary, cwd, env, topology, jobs, or policy change produces different signature.
- [x] Secret environment values participate only in hash input and never appear in display/persisted readable metadata.
- [x] Pure module imports no filesystem, clock, control, executor, or Pi concerns.

## Completed

- `src/duration_history.rs` (pure): `DurationHistory` keyed by signature; retention 20, median (mean of two middles for even), nearest-rank p90, `clamp(max(floor, 10s, p90*1.5+2s), 15m)` with saturating overflow; confidence none/low/medium/high at 0/1-2/3-9/10+; failures + cancelled/superseded/timed-out tracked separately and excluded. 15 tests.
- `src/plan.rs`: `ExecutionSignature` (sha256 hex, Display-safe) + `RunPlan::execution_signature(jobs, fail_fast)` via canonical length-prefixed encoder (schema version, jobs, fail-fast, stage tags, group/occurrence identity, shell-vs-argv tags, cwd, env k/v hashed, BTreeMap order-insensitive). 8 signature tests incl. secret redaction.
- Contract doc tweak: §2 median wording locked to match fixture matrix (mean of two middle values for even counts).

## Notes

