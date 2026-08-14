---
id: TASK-0053
title: Persist bounded run duration history outside the workspace
status: done
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

- [x] Temp-directory tests first cover missing, valid, corrupt, wrong-version, oversized, truncated, permission failure, and atomic replacement paths.
- [x] Default path is `${XDG_STATE_HOME:-~/.local/state}/funzzy/workspaces/<workspace-hash>/run-durations-v1.json`.
- [x] Workspace hash uses canonical root and state schema version; paths are not exposed in protocol.
- [x] Store enforces bounded profiles and samples before allocation/write.
- [x] Writes use 0600 file permissions and atomic temp-file replacement with documented durability behavior.
- [x] Corrupt state is quarantined or replaced with one warning and empty recovery; watcher startup remains usable.
- [x] Concurrent-process writer policy is explicit and tested; no silent JSON corruption or unbounded retry.
- [x] Persistence uses insertion sequence rather than wall-clock recency and cannot trigger watched filesystem events.

## Completed

- `src/duration_store.rs` (new): XDG state path resolution (`default_state_dir`, `workspace_hash` sha256 over canonical root + schema version, `state_file_path`), `DurationStore::load`/`save`. Versioned `StoredState` (schema 1); oversized file rejected pre-decode (64 MiB cap); corrupt/truncated/wrong-version/oversized-profile files quarantined to `.corrupt` with one warning + empty recovery; `0600` via create_new+mode; atomic temp+fsync+rename; single-writer policy (last-rename-wins, no lock/retry); MAX_PROFILES bound enforced on both save and load.
- `src/duration_history.rs`: crate-internal `ProfileSnapshot` + `snapshot()`/`from_snapshot()` accessors (serde-free, retention re-enforced on decode).
- 15 store tests: missing, round-trip, corrupt, wrong-version, oversized, truncated, permission, 0600 perms, atomic replacement (no temp leftover), profile-count reject, retention-bound reject, hash stability/scoping, outside-workspace path, XDG/HOME fallback.

## Notes

