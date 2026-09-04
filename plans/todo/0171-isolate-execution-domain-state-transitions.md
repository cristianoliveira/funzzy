---
id: TASK-0171
title: Isolate execution domain state transitions from process adapters
status: doing
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

- [ ] Define domain transition inputs/outputs for start, readiness promotion, success, failure, timeout, cancellation, replacement, and shutdown.
- [ ] Keep domain transition code independent of CLI, filesystem, process execution, control sockets, stdout/logging, and watcher runtime modules.
- [ ] Preserve generation terminal precedence, declaration/group ordering, process-group cleanup, readiness handoff, and pooled-service ownership.
- [ ] Add a deterministic transition matrix using fake ports; no test may require a real child process to prove a domain transition.
- [ ] Retain focused spawned tests for adapter/process-group behavior and prove their observable outcomes are unchanged.
- [ ] Re-run module graph, DI, SOLID, and complexity analyses to measure boundary and score improvements.

## Verification

Run transition unit tests, executor/service/control integration tests, timeout/cancellation/reload tests, serial and parallel final gates, and `make lint`. Confirm no domain-to-infrastructure imports.
