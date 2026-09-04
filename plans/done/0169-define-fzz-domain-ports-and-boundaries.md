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
- [x] Add a dependency-check test that recursively enumerates every `src/domain` Rust file; it rejects direct, aliased, grouped/multiline, and `super` imports plus fully-qualified `crate::…`/`super::…` references outside comments, strings, and `#[cfg(test)]` items.
- [x] Prove existing CLI, watcher, control, and process behavior remains unchanged.
- [x] Record rejected abstractions and why they do not represent real boundaries.

## Verification

- Red then green: `cargo test --test domain_boundaries` failed while `domain` and `domain/ports.rs` were public, then passed (4 tests) after unused ports were removed and the boundary was made private.
- The token-aware guard walks all `src/domain/**/*.rs` files, checks the pure `rules`/`plan`/`template`/`service_lifecycle` foundations, and finds both import trees and qualified `crate`/`super` paths while ignoring comments, strings, and only the exact `#[cfg(test)]` item.
- Its mutation tests cover direct, aliased, grouped/multiline, `super`, a later production item after `#[cfg(test)]`, and an exact compiling `crate::cmd::execute()` reference.
- Focused: `cargo test --test domain_boundaries` passed (6); `cargo test --lib` passed (877); formatting and diff checks passed.
- Funzzy full unit watcher gate: generation 36 passed in 72979ms. Fresh integration watcher gate: generation 37, `integration @agent-final`, passed in 407700ms with fingerprint `fcf3220d060b`.
- AST module graph scanned 57 Rust files. It found only the pre-existing `executor` ↔ `stdout` cycle; the private `domain` marker has no outgoing edge. Boundary test complexity is 1.5 average with no high-complexity function; its two SRP findings are test-helper heuristics. The private marker has zero DI violations (the analyzer's no-injection style result is not a violation).

## Handoff

TASK-0170 may now separate YAML decoding from domain validation. TASK-0171 may now consume the port boundary while extracting execution transitions. TASK-0172 and TASK-0173 remain blocked by their declared dependencies.
