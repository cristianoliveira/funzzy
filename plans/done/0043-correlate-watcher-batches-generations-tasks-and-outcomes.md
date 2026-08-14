---
id: TASK-0043
title: Correlate watcher batches generations tasks and outcomes
status: done
depends_on: [TASK-0022, TASK-0025]
priority: high
tags: [rust, control-socket, events, identity, determinism, tdd]
---

# Correlate watcher batches generations tasks and outcomes

## Problem
Filesystem events, scheduled runs, task output, and control status need stable shared identities; without them an agent cannot connect its edit to exact execution and result.

## Context

Introduce typed monotonic IDs at domain boundaries rather than deriving identity from timestamps, command strings, or vector positions. IDs are unique within watcher instance; restart changes instance ID.

## Acceptance criteria

- [x] Tests first cover native batches, synthetic emit, init, exact target run, parallel tasks, restart replacement, config reload, and watcher restart.
- [x] One normalized event batch maps to zero or one generation according to scheduling policy and retains complete changed-path set.
- [x] Generation carries trigger/batch relation, predecessor, and superseded-by identity where applicable.
- [x] Task and group-occurrence IDs remain stable from run plan through process/output/outcome/control serialization.
- [x] IDs are deterministic monotonic values within instance and never reused after terminal outcome.
- [x] Snapshot is internally consistent from one state read; fields cannot mix generations.
- [x] Existing control fields remain backward compatible while correlation fields are additive.
- [x] Verbose diagnostics and parallel outcomes consume same typed identities, not duplicate counters.

## Notes

