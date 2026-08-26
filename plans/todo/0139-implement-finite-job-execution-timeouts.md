---
id: TASK-0139
title: Implement finite-job execution timeouts
status: todo
depends_on: [TASK-0138]
priority: normal
tags: [rust, config, executor, timeout, process, control-socket, tdd]
---

# Implement finite-job execution timeouts

## Problem

Funzzy needs to terminate and reap a finite job when its configured execution deadline expires while preserving exact-generation, output, cancellation, and compatibility guarantees.

## Context

Implement only the approved TASK-0138 contract. Timeout is executor policy frozen into the generation, not a presentation-layer timer and not an alias for control-client await timeout.

## Acceptance criteria

- [ ] Write failing tests first for parsing/defaults, valid/invalid bounds, natural exit, timeout, cancellation-before-timeout, timeout/exit race, and descendant cleanup.
- [ ] Parse and validate the approved per-job timeout through canonical config model, schema/catalog, rendered configuration, and semantic revision hash.
- [ ] Carry the frozen effective timeout through run plans and task context without reading reloaded config mid-generation.
- [ ] Enforce the deadline in the shared finite-task executor so local, blocking watch, restart-capable watch, sequential, and parallel paths cannot diverge.
- [ ] On timeout, terminate and reap the complete process group using existing graceful-shutdown/escalation ownership.
- [ ] Publish the approved distinct task/generation timeout state and retain bounded output/duration evidence.
- [ ] Preserve exact cancel/supersede semantics and ensure a stale timeout cannot affect a newer generation or reused process identifier.
- [ ] Apply the approved recovery, hook, fail-fast, and duration-history behavior consistently.
- [ ] Keep jobs without timeout and managed services behaviorally unchanged.
- [ ] Update additive control capability/schema facts and coordinate `pi-watcher` types/decoders when required by the contract.
- [ ] Use deterministic injected timing or synchronization; avoid threshold-sensitive sleeps.

## Verification focus

Keep pure parser/state/race tests beside owning modules. Spawned process-tree and control-socket behavior belongs to TASK-0140.
