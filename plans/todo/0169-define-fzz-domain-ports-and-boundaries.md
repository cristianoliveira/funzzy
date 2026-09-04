---
id: TASK-0169
title: Define FZZ domain ports and dependency boundaries
status: todo
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

- [ ] Inventory domain candidates and infrastructure adapters in `src/`, with named allowed dependency directions.
- [ ] Define minimal ports for clock/time, filesystem/path observation, process execution, output/event publication, and control transport only where domain behavior needs them.
- [ ] Keep ports in a domain-facing module with no imports from CLI, filesystem/process crates, control, stdout/logging, or watcher runtime modules.
- [ ] Add a dependency-check test or static check that fails when domain modules import infrastructure modules.
- [ ] Prove existing CLI, watcher, control, and process behavior remains unchanged.
- [ ] Record rejected abstractions and why they do not represent real boundaries.

## Verification

Use module graph and import checks, domain unit tests with fake ports, `cargo test --lib`, integration tests, and `make lint`. Re-run SOLID/DI scans and document the baseline delta.
