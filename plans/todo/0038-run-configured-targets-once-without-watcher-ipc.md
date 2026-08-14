---
id: TASK-0038
title: Run configured targets once without watcher IPC
status: doing
depends_on: [TASK-0017, TASK-0026]
priority: high
tags: [rust, cli, workflow, run, ci, tdd]
---

# Run configured targets once without watcher IPC

## Problem
Users and CI cannot execute the exact configured workflow locally as a finite command unless a watcher and control socket are already running.

## Context

Add finite local `fzz run TARGET`; distinguish from `fzz control run TARGET`, which requests work from existing watcher.

## Acceptance criteria

- [x] Black-box tests cover exact target, tag/matching contract, missing/ambiguous target, success, failure, fail-fast, parallel groups, and Ctrl-C.
- [x] Uses same config loader, planner, executor, context, output, and exit outcome as watched runs.
- [x] Starts no watcher or control socket.
- [x] CLI help clearly distinguishes local `run` from remote `control run`.
- [x] Exit status reflects combined configured task outcome.
- [x] Optional path input/filter semantics are either explicitly supported or rejected.
- [x] CI example demonstrates parity between manual and watched workflow.

## Notes

Concurrency cap is `on.concurrency`; `jobs` remains available for future task-to-job terminology.

