---
id: TASK-0073
title: Run one exact control generation with sequential override
status: done
depends_on: [TASK-0071, TASK-0072, TASK-0044, TASK-0047]
priority: high
tags: [rust, control-socket, concurrency, capabilities, freshness, tdd]
---

# Run one exact control generation with sequential override

## Problem
Agents primarily verify through running watcher, so local sequential mode alone cannot compare same control target with correlated freshness, output, cancellation, and duration profiles.

## Context

Extend `run` request additively with explicit execution override. Worker request owns per-generation effective concurrency rather than mutating watcher-global executor/config.

## Acceptance criteria

- [ ] Protocol tests first cover absent/default, sequential true, false/no-op, malformed type, unsupported capability, wait, timeout, cancel, supersede, and disconnect.
- [ ] `fzz control|ctl run TARGET --sequential` sends additive typed parameter and schedules exactly one generation with effective concurrency one.
- [ ] Native filesystem/emit/init and later control runs without override retain configured concurrency.
- [ ] Run acknowledgement, atomic wait, subscription, terminal snapshot, output retrieval, and cancellation preserve exact generation identity.
- [ ] Snapshot/result carries configured/effective/source fields fixed at run start and estimate selected from sequential execution signature.
- [ ] Capabilities declares sequential run override and protocol/schema compatibility policy; old clients ignore new snapshot fields.
- [ ] Unsupported old server returns actionable compatibility error before scheduling any work; client never strips flag and retries parallel.
- [ ] One request cannot mutate or race another queued/running generation's policy.
- [ ] Both wait/restart busy policies remain deterministic and process descendants are reaped.

## Notes

