---
id: TASK-0090
title: Swap valid watch roots and execution policy without process exit
status: done
depends_on: [TASK-0089, TASK-0086, TASK-0035, TASK-0040, TASK-0041]
priority: high
tags: [rust, watcher, config, reload, executor, services, tdd]
---

# Swap valid watch roots and execution policy without process exit

## Problem
A valid candidate must update matching roots jobs concurrency debounce ignore hooks and services atomically while preserving PID and avoiding event-loss gaps or duplicated generations.

## Context

Use prepare→commit→retire transaction. Added resources become ready before pointer swap; obsolete resources retire only after commit.

## Acceptance criteria

- [x] Tests first cover root add/remove/overlap, job add/remove/rename, matching/ignore, concurrency, debounce, backend, hooks/output policy, managed service signature, and socket path changes.
- [x] Candidate registers all added native/poll roots and starts required backend/control resources before commit; any failure invokes invalid fatal shutdown without partial live mutation.
- [x] Commit atomically swaps runtime config and routes later batches to new revision; obsolete roots/backend resources retire after boundary without event-loss gap.
- [x] Duplicate events observed by overlapping old/new roots are normalized once with revision/batch identity.
- [x] Active finite tasks keep old revision and complete unless existing busy/cancellation policy explicitly applies; valid config save alone does not kill them.
- [x] Managed services unchanged by execution signature remain owned; changed/removed services receive graceful replacement/removal with bounded kill/reap and new services start under new revision.
- [x] Concurrency/policy changes affect only newly planned generation and never resize currently running group inconsistently.
- [x] Control socket path change uses bind-new-before-retire-old handoff or equivalent safe strategy while process remains alive; bind failure takes fatal path.
- [x] Config watcher remains anchored to parent so atomic replace/delete/recreate is observed after root swap.
- [x] Logging truncate-on-change occurs only for committed valid semantic reload and preserves deterministic notice order.

## Notes

Implementation (TDD, all green: 595 lib tests + full integration suite + docs-drift + nix):

- **Prepare→commit→retire** (`reload_coordinator.rs`): `begin` registers ADDED roots on the live backend before any shared mutation (failure → fatal, nothing mutated); `commit` atomically swaps the shared `Watches` + worker revision + concurrency bound; `retire` unregisters REMOVED roots after the boundary. `ReloadTransaction` carries the root diff + old/new service sets + candidate socket.
- **AC7** (`executor.rs`, `workers.rs`): the executor concurrency bound is an `Arc<AtomicUsize>` shared with the worker; `commit` calls `worker.set_concurrency` so only newly planned generations use the new bound (running groups keep their frozen `stage_limit`).
- **AC8** (`watch_loop.rs`, `app.rs`, `config_revision.rs`): socket path is part of the semantic surface (a socket move is a real revision); `SocketSwapper` binds the NEW socket before commit (failure fatal) and retires the OLD after; wired through the strategy's `prepare_socket_swap`/`retire_socket_swap`.
- **AC9** (`app.rs`): the reload watcher anchors to the config paths' PARENT directories and filters events to the canonical config filenames, so atomic rename-replace/delete-recreate survive any root swap.
- **AC10** (`app.rs` + `tests/watching_with_log_file.rs`): truncate-on-change fires only after a committed valid semantic reload; comment-only (NoOp) saves no longer truncate (updated the stale test to assert the contract).
- **AC6** (`config_revision.rs`, `executor.rs`, `workers.rs`, `reload_coordinator.rs`): per-service signature (secret-safe, semantic) from the rule; `Executor::reconcile_services` stops changed/removed services gracefully by name; `Worker::start_services`/`append_plan` starts new/changed services under the new revision appended to the active generation (unchanged stay owned); diff driven by the coordinator at commit.
- **Test matrix** (`tests/config_reload_matrix.rs`, 7 integration tests): root remove, root overlap normalize-once, job rename, ignore-on-reload, active finite task survives save, service signature replace, socket path handoff.
