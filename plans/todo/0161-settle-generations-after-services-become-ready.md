---
id: TASK-0161
title: Settle generations after services become ready
status: todo
depends_on: [TASK-0160]
priority: high
tags: [rust, services, readiness, executor, control-socket, tdd]
---

# Settle generations after services become ready

## Problem

Healthy managed services currently keep their generation active forever, so the watcher cannot publish a truthful terminal pipeline result while those services continue running.

## Context

Implement only the lifecycle, readiness, output, and compatibility decisions approved in TASK-0160.

## Acceptance criteria

- [ ] Write failing tests first for readiness success, failure before readiness, finite sibling failure, post-readiness service exit, restart, cancellation, supersession, reload, and shutdown.
- [ ] Settle a generation exactly once when all finite work and required service readiness conditions reach the approved outcome.
- [ ] Keep ready services managed for their approved lifetime after generation settlement, without leaving the generation in `running`.
- [ ] Preserve process-group ownership, bounded restart behavior, graceful shutdown, escalation, and descendant reaping.
- [ ] Apply the approved lifecycle when later generations select, omit, replace, or fail around an already-running service.
- [ ] Publish generation outcome and live service state as separate, non-contradictory facts in local output, control status/events, retained evidence, and duration reporting.
- [ ] Run success/failure hooks once at the approved settlement boundary; post-settlement service events must not mutate history or duplicate terminal hooks.
- [ ] Freeze applicable service/readiness policy into scheduled work so config reload cannot alter an in-flight decision.
- [ ] Preserve finite-job, timeout, recovery, sequential/parallel, and legacy configuration behavior outside the approved service change.
- [ ] Coordinate additive protocol/capability and pi-watcher decoder changes when required by TASK-0160.
- [ ] Use deterministic synchronization or injected seams instead of threshold-sensitive sleeps.

## Verification focus

Keep state-transition and race coverage beside the owning Rust modules. Spawned watcher, process-tree, and client compatibility proof belongs to TASK-0162.
