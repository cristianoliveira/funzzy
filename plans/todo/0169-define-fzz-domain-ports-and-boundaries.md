---
id: TASK-0169
title: Define FZZ domain ports and dependency boundaries
status: doing
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
- [x] Define minimal ports for clock/time, filesystem/path observation, process execution, output/event publication, and control transport only where domain behavior needs them.
- [x] Keep ports in a domain-facing module with no imports from CLI, filesystem/process crates, control, stdout/logging, or watcher runtime modules.
- [x] Add a dependency-check test or static check that fails when domain modules import infrastructure modules.
- [x] Prove existing CLI, watcher, control, and process behavior remains unchanged.
- [x] Record rejected abstractions and why they do not represent real boundaries.

## Verification

- Red then green: `cargo test --test domain_boundaries` first failed because `funzzy::domain` did not exist, then passed (2 tests) after the port contracts and static dependency guard were added.
- Focused domain unit tests: `cargo test domain --lib` passed (2 tests); formatting and diff checks passed.
- Funzzy final integration watcher gate: generation 19, `integration @agent-final`, passed in 409438ms with fingerprint `d40649343b4d`. The preceding full generation 18 flaked in the pre-existing native watcher test `newly_created_file_under_existing_watched_dir_triggers_job`; its focused serial rerun passed in 1.86s.
- AST module graph scanned 58 Rust files. It found the pre-existing `executor` ↔ `stdout` cycle; the new `domain/mod` has exactly one outgoing edge to `domain/ports` and no cycle.
- Domain-candidate SOLID scan reports 11 existing SRP findings in `plan`/`watches` and no DIP findings. `domain/mod` + `domain/ports` DI scan reports 0 violations and 0 injections; their average complexity is 1 with no high-complexity function.

## Handoff

TASK-0170 may now separate YAML decoding from domain validation. TASK-0171 may now consume the port boundary while extracting execution transitions. TASK-0172 and TASK-0173 remain blocked by their declared dependencies.
