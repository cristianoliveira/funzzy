---
id: TASK-0027
title: Execute independent tasks with bounded concurrency
status: todo
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

- [ ] Deterministic tests first use barriers/fake process runner to prove overlap without sleeps.
- [ ] Number of simultaneously active tasks never exceeds configured limit or current named-group size.
- [ ] Commands within same task never overlap and retain declared order.
- [ ] Ungrouped tasks never overlap; only tasks in same named contiguous group occurrence may overlap.
- [ ] Next task/group never starts before all tasks in previous parallel group terminate.
- [ ] Reusing same group name after a barrier never reconnects the two occurrences.
- [ ] Parallel-group start/completion order is not part of public contract; tests assert membership, barriers, and bounds rather than incidental order.
- [ ] One task failure does not corrupt sibling state; fail-fast follows TASK-0024 contract.
- [ ] Restart cancellation reaches every active child/process group, skips queued work, and reaps all children before replacement generation starts.
- [ ] Spawn failure occupies and releases one slot correctly and is reported as task failure.
- [ ] Concurrency limit one is behaviorally equivalent to sequential executor.
- [ ] Scheduler does not busy-spin and uses bounded channels/state.

## Notes

