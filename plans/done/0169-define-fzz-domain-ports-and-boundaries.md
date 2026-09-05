---
id: TASK-0169
title: Define FZZ domain ports and dependency boundaries
status: done
depends_on: []
priority: high
tags: [architecture, domain, ports, dependency-direction]
---

# Define FZZ domain ports and dependency boundaries

## Problem

FZZ domain behavior currently depends indirectly on runtime concerns such as process execution, filesystem access, output, and watcher/control orchestration. Without an explicit dependency rule, later refactors can move code mechanically while preserving the coupling.

## Desired outcome

Document and enforce a dependency direction in which domain planning, rules, generations, and outcomes depend only on domain types and ports. CLI, filesystem, process, control-socket, stdout/logging, and watcher adapters depend on the domain—not the reverse.

## Acceptance criteria

- [x] Inventory domain candidates and infrastructure adapters in `src/`, with named allowed dependency directions.
- [x] Define the minimal port introduction points for clock/time, filesystem/path observation, process execution, output/event publication, and control transport; publish no port before its first real consumer and adapter.
- [x] Keep the private domain-boundary module free of imports from CLI, filesystem/process crates, control, stdout/logging, or watcher runtime modules.
- [x] Add a dependency-check test that recursively enumerates every `src/domain` Rust file; it rejects direct, aliased, grouped/multiline, and `super` imports plus fully-qualified `crate::…`/`super::…` references while correctly handling comments (including nested blocks), strings, Rust lifetimes, and `#[cfg(test)]` items.
- [x] Prove existing CLI, watcher, control, and process behavior remains unchanged.
- [x] Record rejected abstractions and why they do not represent real boundaries.

## Verification

- Red then green: `cargo test --test domain_boundaries` failed while `domain` and `domain/ports.rs` were public, then passed (4 tests) after unused ports were removed and the boundary was made private.
- The token-aware guard walks all `src/domain/**/*.rs` files, checks the pure `rules`/`plan`/`template`/`service_lifecycle` foundations, and finds both import trees and qualified `crate`/`super` paths while handling nested block comments, strings, Rust lifetimes, and only the exact `#[cfg(test)]` item.
- Its mutation tests cover direct, aliased, grouped/multiline, `super`, a later production item after `#[cfg(test)]`, an exact compiling `crate::cmd::execute()` reference, a lifetime before a forbidden path, and a nested comment containing a false forbidden path.
- Focused: `cargo test --test domain_boundaries` passed (8); `cargo test --lib` passed (877); formatting and diff checks passed. Full unit watcher generation 41 passed in 68644ms.
- Fresh integration evidence: generation 42, `integration @agent-final`, passed in 400053ms with fingerprint `aa824bbc9dd9`.
- AST module graph scanned 57 Rust files. It found only the pre-existing `executor` ↔ `stdout` cycle; the private `domain` marker has no outgoing edge. Boundary test complexity is 1.3 average with no high-complexity function; its three SRP findings are test-helper heuristics. The private marker has zero DI violations (the analyzer's no-injection style result is not a violation).

## Current close-out evidence (TASK-0169 slice, commit `828ceaf`)

- Inventory and allowed dependency direction remain documented in `docs/DOMAIN-BOUNDARIES.md` and `.tmp/reports/04-09-26/task-0169-domain-inventory.md`.
- `src/plan.rs` now keeps cwd policy filesystem-free; `src/path_context.rs` is the crate-private filesystem adapter for existence and symlink containment. No clock/process/output/control ports were added.
- Focused tests cover absolute paths, parent traversal, workspace root, missing paths, in-workspace paths, and symlink escape. `plan::tests::` passed 37 and `path_context::tests::` passed 3.
- Static boundary guard passed: `cargo test --test domain_boundaries` (8 tests). Task-context integration passed: `cargo test --features test-integration --test task_execution_context` (2 tests).
- Fresh configured watcher gates for current HEAD passed: format generation 75 (fingerprint `9ae1c747ce47`), unit generation 73 (same fingerprint), and integration generation 74 (same fingerprint).
- The pre-existing dirty `pi-watcher` submodule was not modified by this task.

## Handoff

TASK-0170 may now separate YAML decoding from domain validation. TASK-0171 may now consume the port boundary while extracting execution transitions. TASK-0172 and TASK-0173 remain blocked by their declared dependencies.
