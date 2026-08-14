---
id: TASK-0029
title: Prove parallel execution performance and lifecycle safety
status: todo
depends_on: [TASK-0028]
priority: high
tags: [rust, concurrency, performance, reliability, integration-tests]
---

# Prove parallel execution performance and lifecycle safety

## Problem
Parallel scheduling is only valuable if it demonstrably overlaps independent work without leaking children, racing replacement runs, making tests flaky, or corrupting result reporting.

## Context

Use deterministic overlap assertions as primary proof. Wall-clock benchmark is supporting evidence with generous bounds, not sole correctness test.

## Acceptance criteria

- [ ] Integration tests prove contiguous tasks sharing explicit group name overlap, ordinary flat tasks remain sequential, different/separated group occurrences respect barriers, and commands within each task remain sequential.
- [ ] Active-process high-water mark proves configured bound for limits 1, 2, and greater than task count.
- [ ] Combined results cover all pass, one fail, many fail, spawn failure, fail-fast, cancelled, and superseded generations.
- [ ] Repeated cancellation/replacement leaves no child, process group, forwarding thread, or executor thread behind.
- [ ] Control `run`, synthetic `emit`, filesystem events, and run-on-init all use same parallel engine.
- [ ] Output remains line-safe/task-attributed and control state produces same task-keyed combined result under deliberately reversed completion order; incidental ordering is not asserted.
- [ ] Supporting benchmark demonstrates independent task batch latency approaches slowest task rather than sum, with environment recorded.
- [ ] README/docs explain named contiguous groups, barriers, filtering, `on.concurrency`, failures, restart, output ordering, and migration-free sequential defaults.
- [ ] CPU/process cost and recommended concurrency guidance are documented; no claim that parallelism makes every workload faster.
- [ ] Focused, integration, and platform-relevant verification gates pass repeatedly without flakes.

## Notes

