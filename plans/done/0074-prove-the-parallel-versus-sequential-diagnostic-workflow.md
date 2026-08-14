---
id: TASK-0074
title: Prove the parallel versus sequential diagnostic workflow
status: done
depends_on: [TASK-0072, TASK-0073, TASK-0028]
priority: high
tags: [integration-tests, concurrency, debugging, agents, estimates, reliability]
---

# Prove the parallel versus sequential diagnostic workflow

## Problem
A flag and protocol field do not prove that a failing parallel workflow can be rerun sequentially without changing task selection, hiding failures, contaminating estimates, or overstating race causation.

## Context

Use deterministic synchronization fixtures, not flaky probability or tight wall-clock assertions.

## Acceptance criteria

- [ ] Black-box fixture intentionally fails only when two configured jobs overlap and passes under explicit sequential override.
- [ ] Comparison proves same target membership, commands, cwd/env, barriers, fail-fast policy, and changed set; only effective concurrency differs.
- [ ] Local run, watch session, `control run --wait`, `ctl` alias, and pi-watcher path exercise expected scope.
- [ ] Snapshot/output evidence distinguishes parallel and sequential generations and remains task-attributed under reversed completion.
- [ ] Sequential sample uses separate duration signature/profile and cannot lower/raise parallel recommendation.
- [ ] Unsupported legacy server and malformed override schedule no hidden parallel retry.
- [ ] Cancellation/supersession during sequential comparison leaves no descendants and produces exact terminal reason.
- [ ] Agent-facing result uses `parallel-sensitive` only for parallel fail + comparable sequential pass and includes caveat that causation is unproven.
- [ ] Documentation gives copyable manual and agent comparison recipe plus limitations.

## Notes

