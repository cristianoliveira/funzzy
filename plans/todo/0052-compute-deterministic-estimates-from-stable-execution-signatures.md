---
id: TASK-0052
title: Compute deterministic estimates from stable execution signatures
status: todo
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

- [ ] Tests first cover empty, one/two samples, odd/even median, nearest-rank p90, retention eviction, outlier, overflow, floor, cap, and confidence boundaries.
- [ ] Last N successful durations produce deterministic `typicalMs`, `upperMs`, recommendation, samples, confidence, and source.
- [ ] Failed durations are separate diagnostics; cancelled/superseded/timed-out samples cannot lower success estimate.
- [ ] Signature canonicalizes stage/barrier order, task/group identity, shell versus argv boundaries, cwd, declared env, jobs, fail-fast, and schema version.
- [ ] Equivalent environment map insertion order produces same signature.
- [ ] Command, argv boundary, cwd, env, topology, jobs, or policy change produces different signature.
- [ ] Secret environment values participate only in hash input and never appear in display/persisted readable metadata.
- [ ] Pure module imports no filesystem, clock, control, executor, or Pi concerns.

## Notes

