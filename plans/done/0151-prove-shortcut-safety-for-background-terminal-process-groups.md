---
id: TASK-0151
title: Prove shortcut safety for background terminal process groups
status: done
depends_on: []
priority: high
tags: []
---

# Prove shortcut safety for background terminal process groups

## Problem
The TASK-0150 stdin shortcut reader stopped an entire cargo test process group with a terminal job-control signal when a spawned fzz inherited a controlling terminal but was not its foreground process group. Reading can raise SIGTTIN, while raw-mode terminal changes can raise SIGTTOU first. The production guard landed without a deterministic regression test, so the hang can silently return.

## Context

TASK-0150 added Ctrl-G shortcut input. Commit `fd5d45f` prevents reads when a TTY's foreground process group differs from the current process group, but independent QA rejected closure without an exact regression test.

## Acceptance criteria

- [x] A deterministic Unix integration test uses a controlling PTY and a real background process group to reproduce the former SIGTTIN/SIGTTOU stop.
- [x] The test fails when the foreground-process-group guard is removed and passes with it.
- [x] The test uses bounded polling, cleans up all child processes, and cannot hang the test runner.
- [x] Existing piped blocking, non-blocking, busy-latch, Ctrl-C, status, and output behavior remains passing.
- [x] Independent QA accepts the evidence; final watcher gate passes with an unchanged worktree fingerprint.

## Notes

- Production guard: `fd5d45f`.
- Initial QA report: `/Users/cristianoliveira/.agents/reports/29-08-26/task-0150-final-independent-qa.md`.

