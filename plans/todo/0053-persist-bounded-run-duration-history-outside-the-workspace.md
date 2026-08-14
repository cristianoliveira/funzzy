---
id: TASK-0053
title: Persist bounded run duration history outside the workspace
status: todo
depends_on: [TASK-0052]
priority: high
tags: [rust, infra, xdg, persistence, bounds, tdd]
---

# Persist bounded run duration history outside the workspace

## Problem
In-memory estimates disappear on watcher restart, while storing history inside watched worktree would dirty repository and can trigger feedback loops.

## Context

Add injected state-path adapter under XDG state. Keep estimator independent from serde/filesystem. No history file may live below watched workspace.

## Acceptance criteria

- [ ] Temp-directory tests first cover missing, valid, corrupt, wrong-version, oversized, truncated, permission failure, and atomic replacement paths.
- [ ] Default path is `${XDG_STATE_HOME:-~/.local/state}/funzzy/workspaces/<workspace-hash>/run-durations-v1.json`.
- [ ] Workspace hash uses canonical root and state schema version; paths are not exposed in protocol.
- [ ] Store enforces bounded profiles and samples before allocation/write.
- [ ] Writes use 0600 file permissions and atomic temp-file replacement with documented durability behavior.
- [ ] Corrupt state is quarantined or replaced with one warning and empty recovery; watcher startup remains usable.
- [ ] Concurrent-process writer policy is explicit and tested; no silent JSON corruption or unbounded retry.
- [ ] Persistence uses insertion sequence rather than wall-clock recency and cannot trigger watched filesystem events.

## Notes

