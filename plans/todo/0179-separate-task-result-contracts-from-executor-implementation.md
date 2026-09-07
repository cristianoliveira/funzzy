---
id: TASK-0179
title: Separate task result contracts from executor implementation
status: doing
depends_on: []
priority: high
tags: [rust, executor, output, architecture]
---

# Separate task result contracts from executor implementation

## Problem
stdout imports executor-owned task snapshots while executor calls stdout. Shared result types create an avoidable cycle between presentation and execution.

## Context

`src/executor.rs` defines `TaskSnapshot`/`TaskState` and imports `stdout`; `src/stdout.rs::job_duration_rows` imports those executor types. TASK-0173 explicitly left this cycle unchanged after isolating control/output query seams.

## Scope and approach

- Inventory result-type consumers and current serialization before editing.
- Move the two shared result types into a focused contract owner with no executor or stdout dependency. Keep them at the application/protocol boundary rather than moving serde into pure lifecycle policy merely for directory symmetry.
- Re-export both types from `executor` so current public Rust paths remain valid.
- Make stdout and internal result consumers import the actual owner where appropriate.
- Add a narrow dependency guard against restoring the direct executor/stdout cycle.

## Acceptance criteria

- [ ] `TaskSnapshot` and `TaskState` each have one definition outside executor implementation.
- [ ] Existing `funzzy::executor` type paths compile through compatibility re-exports.
- [ ] Serialization is unchanged, including camelCase fields, lowercase states such as `timedout`, and skipped declaration position.
- [ ] Empty and populated human duration tables retain their current behavior.
- [ ] The dependency scan no longer reports the direct executor/stdout cycle, confirmed by source inspection.
- [ ] Rust control/client and Pi decoder expectations remain compatible; no submodule change is required unless a real mismatch is found.

## Verification

Characterize serialization and presentation before the move. Run focused stdout, snapshot, watcher-state, event-stream, and control tests, plus feature-enabled report/control tests and the fresh watcher final gate. Record actual consumer impact; do not rely on a failed semantic index.

## Progress evidence

- Inventory and pre-move characterization: `.tmp/reports/04-09-26/task-0179-pre-move-inventory.md`.
- Characterization commit `4426abb`: serialization shape test (1 passed) and empty/populated duration-table tests (2 passed) landed before production move.
- Type-owner seam `fa967b4`: `src/task_result.rs` owns both result contracts; `executor` re-exports both compatibility paths; stdout and internal projections import the actual owner. Pi watcher was not changed because it uses independent decoder types.
- Dependency proof: after the move, `ast_module_graph` reports no direct `executor↔stdout` cycle; only pre-existing `cli↔config` remains. `git diff --check` is clean.
- Focused results: executor 67, stdout 2, snapshot 8, watcher_state 9, event_stream 6, duration_recorder 12, control 60, control_client 32; feature `control_output` 11, `control_socket` 14, and serial `finite_job_timeouts` 9; domain boundaries 8. The parallel finite-timeout run exposed two existing process-tree timing failures; both affected tests passed individually and in serial, with no changes to runtime behavior.
- Full consumer and compatibility mapping: `.tmp/reports/04-09-26/task-0179-result-contract-seam.md`.


## Likely files

`src/executor.rs`, `src/stdout.rs`, `src/lib.rs`, one focused contract module, consuming adapters as needed, and boundary/compatibility tests.

## Non-goals

No public wire changes, new event kinds, removal of existing exports, or general-purpose shared-types module.
