---
id: TASK-0035
title: Support managed long-running service tasks
status: done
depends_on: [TASK-0030, TASK-0027]
priority: normal
tags: [rust, workflow, service, process, lifecycle, tdd]
---

# Support managed long-running service tasks

## Problem
Development servers have different success, readiness, restart, and shutdown semantics from finite commands, but Funzzy currently treats every command as work that must exit.

## Context

Treat service as explicit task kind, not command that accidentally never exits. Decide whether initial scope needs readiness probe or only spawned/running readiness.

## Acceptance criteria

- [ ] Contract defines service start success, readiness, unexpected exit, restart, failure, and watcher shutdown.
- [ ] Parser rejects incompatible combinations and preserves finite task defaults.
- [ ] Process ownership from TASK-0030 handles signals, grace, escalation, and descendants.
- [ ] Changed generation rebuild/prep ordering relative to service restart is explicit.
- [ ] Status/control reports starting, running, failed, stopping, and stopped states.
- [ ] Parallel group interaction is defined; service cannot hold finite-stage barrier forever.
- [ ] Deterministic tests use fake service/probe clock; integration covers real graceful and forced shutdown.
- [ ] Docs state services are opt-in and not hot code reload.

## Notes

