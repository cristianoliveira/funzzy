---
id: TASK-0006
title: Unify blocking and non-blocking watch orchestration
status: todo
depends_on: [TASK-0001, TASK-0003]
priority: normal
tags: [rust, orchestration, cohesion]
---

# Unify blocking and non-blocking watch orchestration

## Problem

Blocking and non-blocking commands duplicate watch readiness, task selection, init/change handling, templates, output, and fail-fast policy. Features can drift between two execution paths.

## Scope

- `src/cli/watch.rs`
- `src/cli/watch_non_block.rs`
- Shared application watch loop
- Blocking and replacing executor capabilities

## Acceptance criteria

- [ ] One application flow owns filesystem readiness and event-to-run conversion.
- [ ] Blocking and cancellable behavior are injected executor strategies.
- [ ] Init and file-change triggers have one preparation path.
- [ ] Fail-fast semantics remain covered in both modes.
- [ ] Control socket uses run orchestration contract rather than concrete worker internals.
- [ ] Output and CLI compatibility remain unchanged unless explicitly approved.

## Verification

- Shared orchestration has deterministic unit tests.
- Blocking, non-block, fail-fast, init, and control integration tests pass.

