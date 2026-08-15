---
id: TASK-0080
title: Make retained output errors typed and instance exact
status: todo
depends_on: [TASK-0079, TASK-0046]
priority: high
tags: [rust, control-socket, identity, errors, output, tdd]
---

# Make retained output errors typed and instance exact

## Problem
Output retrieval currently uses instance-scoped generation without instance token and maps missing generation/task to generic server errors, so clients cannot distinguish stale watcher, eviction, typo, or protocol failure.

## Context

Generation counter resets per watcher instance. Require exact instance on advanced retrieval while preserving explicit legacy behavior for old clients.

## Acceptance criteria

- [ ] Tests first lock stable typed error codes and structured data for unknown instance/generation/task, eviction, invalid options/cursor, and unavailable registry.
- [ ] Output request validates `instanceToken` against active watcher before registry lookup; stale token cannot read same-number generation from replacement watcher.
- [ ] Missing/legacy instance behavior follows contract/capability and never claims exact freshness.
- [ ] Registry stores/returns exact task ID separately from display name and emits deterministic canonical candidates for unknown task.
- [ ] One unambiguous read-only candidate may be resolved according to contract; multiple/zero candidates return typed error without retrieval.
- [ ] CLI and JSON/TOON render actionable exact retry data without parsing message strings.
- [ ] pi-watcher expected `-32010/-32011` mappings and Rust server codes/fixtures agree; generic `-32000` is reserved for genuine server failure.
- [ ] Restart, generation reuse, cancellation, supersession, retention eviction, and concurrent retrieval race tests fail closed.
- [ ] Existing clients receive additive-compatible response where feasible and capability clearly marks exact-output support.

## Notes
