---
id: TASK-0171
title: Isolate execution domain state transitions from process adapters
status: done
depends_on: [TASK-0169]
priority: high
tags: [architecture, domain, executor, services, state-machine]
---

# Isolate execution domain state transitions from process adapters

## Problem

`src/executor.rs`, `src/workers.rs`, and `src/service_pool.rs` mix scheduling decisions and generation/service state transitions with process spawning, readiness, output, and cleanup. The resulting boundary makes lifecycle changes hard to test without real processes.

## Desired outcome

Represent finite-job and service lifecycle decisions as pure domain transitions. Process execution, clocks, readiness probes, output, and cleanup enter through explicit ports implemented by runtime adapters.

## Acceptance criteria

- [x] Define domain transition inputs/outputs for start, readiness promotion, success, failure, timeout, cancellation, replacement, and shutdown.
- [x] Keep domain transition code independent of CLI, filesystem, process execution, control sockets, stdout/logging, and watcher runtime modules.
- [x] Preserve generation terminal precedence, declaration/group ordering, process-group cleanup, readiness handoff, and pooled-service ownership.
- [x] Add a deterministic transition matrix using fake ports; no test may require a real child process to prove a domain transition.
- [x] Retain focused spawned tests for adapter/process-group behavior and prove their observable outcomes are unchanged.
- [x] Re-run module graph, DI, SOLID, and complexity analyses to measure boundary and score improvements.

## Verification

Run transition unit tests, executor/service/control integration tests, timeout/cancellation/reload tests, serial and parallel final gates, and `make lint`. Confirm no domain-to-infrastructure imports.

## Evidence

- Pure transitions: `src/domain/finite_lifecycle.rs` covers `NotStarted`/start, continue, pass, fail, timeout, cancellation precedence, fail-fast, and recovery eligibility; `src/service_lifecycle.rs` covers readiness promotion/retry/timeout/service exit and command precedence. Tests use semantic values and deterministic fake clocks only.
- Runtime preservation: executor and worker tests cover terminal precedence, declaration/group ordering, process-group shutdown, readiness handoff, replacement, cancellation, reload, and pooled-service ownership. No runtime process type is imported by `src/domain/`.
- Ports: `src/domain/ports.rs::Clock` is the only extracted domain-facing execution port. `ProcessRunner`, `ChildProcess`, and `EventSink` remain application/runtime ports because they expose runtime-specific command, capture, exit-status, signal, cleanup, and event vocabulary; moving them would violate the boundary or merely rename infrastructure.
- Verification: executor unit tests (66 passed), domain boundary tests (8 passed), full watcher unit gate (gen120 passed), integration gate (gen121 passed), `make lint` passed, and `cargo fmt -- --check` passed. Module graph reports no domain-to-infrastructure edge; existing unrelated cycles remain `cli`/`config` and `executor`/`stdout`. DI/SOLID checks for the extracted domain modules report zero violations; complexity analysis reports no high-complexity functions.
