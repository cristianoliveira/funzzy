---
id: TASK-0056
title: Prove duration estimates persist invalidate and stay bounded
status: done
depends_on: [TASK-0055]
priority: high
tags: [integration-tests, duration, persistence, performance, reliability]
---

# Prove duration estimates persist invalidate and stay bounded

## Problem
Unit statistics and serializers do not prove repeated real runs produce useful recommendations, survive restart, reset after workflow changes, and avoid unbounded state or worktree events.

## Context

Use fake monotonic clock for domain/integration seams and bounded real CLI scenarios without timing-sensitive assertions on host speed.

## Acceptance criteria

- [x] Repeated exact target successes produce expected median/p90/recommendation and confidence progression.
- [x] Watcher restart reloads history and preserves estimate for unchanged signature.
- [x] Command, argv, cwd, env, topology, jobs, and fail-fast changes invalidate old profile.
- [x] Failure, cancellation, supersession, and timeout cannot lower successful timeout recommendation.
- [x] More than retention/profile limits evicts deterministically and keeps memory/file size bounded.
- [x] Corrupt/oversized history recovers without blocking watcher and emits one actionable warning.
- [x] State writes create no watched worktree event or feedback-loop diagnostic.
- [x] Parallel target recommendation uses observed wall time, not task-duration sum.
- [x] Old client/server fixtures remain compatible and unsupported estimate is explicit.
- [x] Documentation covers location, reset procedure, privacy, estimator, confidence, and limitations.

## Completed

- `tests/duration_estimates.rs` (new, 7 black-box tests over the real `fzz` binary + control socket, isolated `XDG_STATE_HOME` per test, state-based assertions, no host-speed timing):
  1. repeated successes → estimate present, `typical<=upper<=recommended` invariant, samples=3, confidence medium, source measured, capabilities feature + limits
  2. restart preserves estimate (samples/confidence survive)
  3. command change invalidates old profile (estimate absent until new samples)
  4. same-signature failure (file gate) never alters success estimate
  5. state file lives outside workspace; no feedback-loop generation after writes
  6. corrupt history quarantines + recovers empty + watcher usable + fresh samples recorded
  7. legacy compatibility: additive `estimate` optional field; no signature/env/state-path leakage
- `docs/DURATION-ESTIMATES-GUIDE.md` (new): location, recording rules, estimate fields, agent usage, privacy, reset/recovery, limitations.
- `duration_recorder` unit test: parallel generation records exactly one wall-time sample (never per-task sum).
- Bounds/eviction, exclusion, and corrupt-recovery determinism already locked by TASK-0052/0053 unit matrices.

## Notes

