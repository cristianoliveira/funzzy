---
id: TASK-0046
title: Cancel an exact generation through the control socket
status: done
depends_on: [TASK-0030, TASK-0044]
priority: high
tags: [rust, cli, control-socket, cancellation, process, tdd]
---

# Cancel an exact generation through the control socket

## Problem
Agents need a bounded escape from obsolete or stuck work, but cancellation must target exact generation and complete process tree rather than race with replacement work.

## Context

Cancellation is compare-and-act on generation identity. It must never cancel whatever happens to be current after request was formed.

## Acceptance criteria

- [ ] Tests first cover queued, running serial, running parallel, already terminal, unknown, superseded, repeated request, generation race, and socket disconnect.
- [ ] `fzz control cancel --generation ID [--wait --timeout DURATION]` requires explicit generation.
- [ ] Server atomically verifies identity before requesting cancellation and returns no-op for same terminal state.
- [ ] Cancellation uses TASK-0030 process ownership and waits/reports graceful versus escalated termination.
- [ ] Queued/later tasks follow fail-fast/cancel contract and final outcome identifies cancelled tasks.
- [ ] Replacement/newer generation cannot be affected by stale cancel request.
- [ ] `--wait` reuses TASK-0044 and returns exact terminal snapshot.
- [ ] Protocol remains additive and Pi watcher decoder/tests are coordinated.

## Notes

