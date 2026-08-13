---
id: TASK-0003
title: Inject workspace root into watch planning and execution
status: todo
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

- [ ] Composition root resolves workspace root once.
- [ ] Watch planning receives explicit root/path context.
- [ ] Command template preparation uses same injected root.
- [ ] Core tests do not mutate process current directory.
- [ ] Absolute, relative, and outside-workspace paths retain behavior.
- [ ] A CLI convenience constructor may remain only at outer boundary.

## Verification

- Unit tests cover roots with spaces and relative/absolute paths.
- Existing path-template and configured-rule integration tests pass.

