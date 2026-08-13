---
id: TASK-0022
title: Route synthetic path events through the control socket
status: todo
depends_on: [TASK-0021, TASK-0018]
priority: normal
tags: [rust, cli, ipc, control-socket, workflow, pi-watcher, tdd]
---

# Route synthetic path events through the control socket

## Problem
External integrations may know that a logical project path changed without producing a reliable filesystem notification, but the current socket can only trigger targets by name and therefore couples producers to workflow configuration.

## Context

Add `fzz control emit PATH` and corresponding JSON-RPC `emit` method. Producer reports a path; Funzzy owns routing through existing change/ignore rules. See `.tmp/reports/13-04-26/control-emit-problem.md`.

This must feed shared event-to-run policy rather than creating second matcher or executor inside `control.rs`.

## Acceptance criteria

- [ ] Tests first cover relative path, absolute path, ignored path, unmatched path, malformed/empty path, deleted/nonexistent path, busy watcher, and notification form without an `id`.
- [ ] JSON-RPC `emit` accepts one normalized path and returns deterministic matched-task/run identity defined in TASK-0014.
- [ ] `fzz control emit PATH` sends request to running watcher and can use same `--wait`/timeout semantics as `control run` when work is scheduled.
- [ ] Emitted path follows same normalization, change matching, ignore precedence, task ordering, templates, `run_on_init` exclusions, and busy-run policy as native change event.
- [ ] Unmatched/ignored result is explicit and schedules no generation.
- [ ] Missing path does not need to exist, so deletion and remote logical events remain representable.
- [ ] Transport validation remains separate from matching and execution policy.
- [ ] Existing `status`, `targets`, and `run` wire contracts remain unchanged.
- [ ] Rust control integration tests and Pi watcher protocol/client tests are synchronized for added method.
- [ ] Documentation states that `emit` reports a path change; it does not mutate filesystem or provide generic event bus.

## Notes

Security boundary remains permission-restricted Unix socket. Path normalization and template handling must be identical to native filesystem events; do not introduce IPC-only interpolation behavior.

