---
id: TASK-0056
title: Prove duration estimates persist invalidate and stay bounded
status: todo
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

- [ ] Repeated exact target successes produce expected median/p90/recommendation and confidence progression.
- [ ] Watcher restart reloads history and preserves estimate for unchanged signature.
- [ ] Command, argv, cwd, env, topology, jobs, and fail-fast changes invalidate old profile.
- [ ] Failure, cancellation, supersession, and timeout cannot lower successful timeout recommendation.
- [ ] More than retention/profile limits evicts deterministically and keeps memory/file size bounded.
- [ ] Corrupt/oversized history recovers without blocking watcher and emits one actionable warning.
- [ ] State writes create no watched worktree event or feedback-loop diagnostic.
- [ ] Parallel target recommendation uses observed wall time, not task-duration sum.
- [ ] Old client/server fixtures remain compatible and unsupported estimate is explicit.
- [ ] Documentation covers location, reset procedure, privacy, estimator, confidence, and limitations.

## Notes

