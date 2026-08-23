---
id: TASK-0129
title: Implement configurable recovery approval timeout
status: done
depends_on: [TASK-0128]
priority: high
tags: [rust, config, executor, recovery, approval, timeout, tdd]
---

# Implement configurable recovery approval timeout

## Problem
Funzzy needs to default-deny unanswered recovery approval after bounded configured duration while preserving cancellation and frozen-generation semantics.

## Context

Use TDD. Timeout is executor policy, not only a UI convenience. Thread frozen duration through configuration snapshot → watch/worker request → `RunMetadata`. Let approval port observe same deadline so TTY polling exits before reading stale input; executor must still bound a misbehaving approval adapter.

## Acceptance criteria
- [ ] Write failing tests first for default, valid configured duration, invalid/zero value, timeout decision, approval before deadline, and cancellation before deadline.
- [ ] Parse and validate `execution.recovery_timeout` through canonical option catalog/schema using existing duration syntax, default `60s`.
- [ ] Include effective timeout in immutable `RuntimeConfig` and semantic revision hash; hot reload changes only later generations.
- [ ] Thread frozen timeout through blocking workflow and restart-capable worker into exact generation metadata.
- [ ] Add explicit `ApprovalDecision::TimedOut` and actionable `approval timeout` phase/diagnostic.
- [ ] Bound executor wait even when injected approval adapter never responds.
- [ ] Replace blocking `stdin.read_line()` after readiness with cancellation-safe bounded input; partial input without newline must not strand approval thread or retain stdin after timeout.
- [ ] Make TTY adapter stop polling at deadline without consuming late input as stale/current approval.
- [ ] Preserve cancellation precedence and ensure timeout never spawns recovery or verification commands.
- [ ] Cover prompt, skip, no-TTY, timeout, and approved paths without wall-clock-flaky assertions; inject time/deadline seams where needed.
- [ ] Keep ordinary jobs and configurations without recovery behaviorally unchanged.

## Notes

Do not solve this only in `watcher_verify`: watcher tools already have caller timeouts, but generation itself remains non-terminal and other agents keep waiting.

