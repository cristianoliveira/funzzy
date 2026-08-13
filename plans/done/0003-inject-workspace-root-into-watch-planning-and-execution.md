---
id: TASK-0003
title: Inject workspace root into watch planning and execution
status: done
depends_on: []
priority: normal
tags: [rust, determinism, dependency-injection]
---

# Inject workspace root into watch planning and execution

## Problem

`Watches::new`, command scheduling, and config path preparation read process current directory independently. Core behavior therefore depends on hidden mutable global state and is harder to test deterministically.

## Scope

- `src/watches.rs`
- `src/workers.rs`
- Composition root and path-related tests

## Acceptance criteria

- [x] Composition root resolves workspace root once (`src/app.rs` resolves `workspace_root` before command dispatch).
- [x] Watch planning receives explicit root/path context (`Watches::with_root`).
- [x] Command template preparation uses same injected root (`Worker::with_root`; blocking watch uses `watches.root()`).
- [x] Core tests do not mutate process current directory (new tests use temp roots, including roots with spaces).
- [x] Absolute, relative, and outside-workspace paths retain behavior.
- [x] A CLI convenience constructor may remain only at outer boundary (`Watches::new`/`Worker::new` kept for tests and discovery).

## Verification

- [x] Unit tests cover roots with spaces and relative/absolute paths.
- [x] Existing path-template and configured-rule integration tests pass.
