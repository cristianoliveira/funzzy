---
id: TASK-0027
title: Execute independent tasks with bounded concurrency
status: done
depends_on: [TASK-0026, TASK-0030]
priority: high
tags: [rust, concurrency, scheduler, process, tdd]
---

# Execute independent tasks with bounded concurrency

## Problem
Sequential execution makes total feedback latency approach the sum of independent task durations instead of their maximum, but unbounded process spawning would trade latency for instability.

## Context

Use bounded scheduling only within named contiguous parallel-group occurrences. Ungrouped tasks execute one at a time. Executor advances past group barrier only after all selected tasks in current occurrence reach terminal outcome. A task owns at most one active command; next command starts only after prior command succeeds or task policy permits continuation.

## Acceptance criteria

- [x] Deterministic tests first use barriers/fake process runner to prove overlap without sleeps.
- [x] Number of simultaneously active tasks never exceeds configured limit or current named-group size.
- [x] Commands within same task never overlap and retain declared order.
- [x] Ungrouped tasks never overlap; only tasks in same named contiguous group occurrence may overlap.
- [x] Next task/group never starts before all tasks in previous parallel group terminate.
- [x] Reusing same group name after a barrier never reconnects the two occurrences.
- [x] Parallel-group start/completion order is not part of public contract; tests assert membership, barriers, and bounds rather than incidental order.
- [x] One task failure does not corrupt sibling state; fail-fast follows TASK-0024 contract.
- [x] Restart cancellation reaches every active child/process group, skips queued work, and reaps all children before replacement generation starts.
- [x] Spawn failure occupies and releases one slot correctly and is reported as task failure.
- [x] Concurrency limit one is behaviorally equivalent to sequential executor.
- [x] Scheduler does not busy-spin and uses bounded channels/state.

## Notes

Completed with stage-aware bounded scheduling in the shared executor, task-sequential command state, deterministic fake-child barrier tests, and a one-slot overwrite scheduler for newest-generation replacement. `on.jobs` is wired from configuration; absent values resolve available parallelism once. Path, init, and target filtering preserve original group occurrences. Fresh Funzzy watcher generation 99 passed all configured checks.

