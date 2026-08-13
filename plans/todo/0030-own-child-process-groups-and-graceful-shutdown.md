---
id: TASK-0030
title: Own child process groups and graceful shutdown
status: todo
depends_on: [TASK-0026]
priority: high
tags: [rust, process, signals, reliability, tdd]
---

# Own child process groups and graceful shutdown

## Problem
Funzzy currently signals direct child PIDs, which can leave grandchildren or forwarding threads alive during restart, Ctrl-C, config reload, parallel execution, or future service tasks.

## Context

Create one process-ownership abstraction used by executor. On Unix, each command gets own process group/session so cancellation reaches descendants. Keep platform behavior explicit and testable.

## Acceptance criteria

- [ ] Tests first cover normal exit, spawn failure, graceful cancellation, grace timeout escalation, grandchildren, repeated cancel, and owner drop.
- [ ] Executor signals owned process group rather than only shell PID.
- [ ] Configurable signal and grace duration have safe deterministic defaults.
- [ ] Escalation force-kills after grace period and reports that decision.
- [ ] Restart waits until all children and forwarding threads are reaped before replacement starts.
- [ ] Ctrl-C, config reload, worker drop, fail-fast, and normal shutdown use same ownership path.
- [ ] No zombie/orphan remains in repeated integration test.
- [ ] Unsupported platform behavior fails explicitly or has documented equivalent.

## Notes

