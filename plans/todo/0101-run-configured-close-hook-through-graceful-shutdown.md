---
id: TASK-0101
title: Run configured close hook through graceful shutdown
status: doing
depends_on: [TASK-0100]
priority: high
tags: [rust, watcher, config, hooks, signals, process, tdd]
---

# Run configured close hook through graceful shutdown

## Problem
Current SIGINT, SIGTERM, fatal reload, and watcher teardown paths exit independently, so adding a hook directly to one path would be incomplete, unsafe, and likely to orphan processes.

## Context

Create one idempotent shutdown coordinator owned and wired by `app.rs`. Async signal handler remains self-pipe-only; configured command execution must happen on normal Rust control flow, never inside signal handler. Reuse process runner/ownership instead of direct `Command` spawning.

## Acceptance criteria

- [ ] Write failing unit tests first for parsing, close-hook selection, exactly-once state transition, latest committed reload value, failure, and timeout.
- [ ] Add `close` to canonical `on` option catalog and schema; parser returns typed session hook alongside existing generation hooks without raw YAML access in shutdown code.
- [ ] Separate generation hooks from watcher lifecycle hook in names/types so finite runners cannot accidentally execute close hook.
- [ ] Introduce one thread-safe, idempotent graceful-shutdown coordinator that owns shutdown reason, exit code, active process reaping, close-hook execution, and final cleanup ordering.
- [ ] SIGINT/SIGTERM handler performs only async-signal-safe notification; remove direct configured work and competing `process::exit` paths from signal thread.
- [ ] Normal watch exit and fatal reload route through same coordinator; first shutdown reason wins deterministically and concurrent callers observe same completion.
- [ ] Stop scheduling and close control surface before hook execution; cancel/reap active job and service groups before spawning hook.
- [ ] Execute hook through existing configured shell/process ownership path at workspace root, capture/report failure, enforce bounded grace, and reap hook descendants.
- [ ] Preserve original exit codes regardless of hook result and avoid process-global state leaking between tests.
- [ ] Valid config reload atomically replaces future close hook with rest of committed runtime config; invalid candidate retains last committed hook for fatal shutdown.
- [ ] Verbose/event output identifies lifecycle hook and shutdown reason without generation correlation; normal human output remains bounded.
- [ ] No hook is wired into finite `run` or non-watcher commands.

## Notes

Composition root owns shutdown dependency wiring. Do not add close-hook execution independently to each exit branch.

