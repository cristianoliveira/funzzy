---
id: TASK-0179
title: Separate task result contracts from executor implementation
status: todo
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

## Likely files

`src/executor.rs`, `src/stdout.rs`, `src/lib.rs`, one focused contract module, consuming adapters as needed, and boundary/compatibility tests.

## Non-goals

No public wire changes, new event kinds, removal of existing exports, or general-purpose shared-types module.
