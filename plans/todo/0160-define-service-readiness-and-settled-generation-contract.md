---
id: TASK-0160
title: Define service readiness and settled-generation contract
status: todo
depends_on: []
priority: high
tags: [design, services, readiness, generations, control-socket, determinism]
---

# Define service readiness and settled-generation contract

## Problem

A generation containing a healthy `service: true` job stays `running` for the service's entire lifetime. This makes a successfully started development pipeline look unfinished to humans and agents, even after every finite check has passed.

## Desired outcome

A generation can report terminal success once its finite jobs pass and its included services reach a defined ready state. Those services may remain managed in the background, with their current health and lifecycle reported separately from the immutable generation result.

## Acceptance criteria

- [ ] Define a deterministic service readiness boundary; explicitly decide whether successful spawn is sufficient, a stabilization check is required, or users may configure an explicit readiness signal.
- [ ] Define generation success and failure when finite jobs and services start sequentially or in parallel, including a service that exits before readiness.
- [ ] Define post-readiness service exit and restart semantics without retroactively changing an already-terminal generation.
- [ ] Define watcher ownership for ready services after their starting generation settles, including unrelated generations, service re-inclusion, config reload, replacement, cancellation, and watcher shutdown.
- [ ] Define what happens when a later generation selects the same service, omits it, or fails before the service is replaced.
- [ ] Separate last-generation outcome from current service state in human output, control snapshots/events, retained evidence, duration reporting, and pi-watcher decoding.
- [ ] Define when success/failure/close hooks run and ensure one settled generation cannot emit contradictory terminal hooks later.
- [ ] Define deterministic precedence for readiness, immediate exit, explicit cancellation, supersession, reload, and shutdown races.
- [ ] Record compatibility and protocol-version impact. Existing finite jobs and configurations without services must remain unchanged.
- [ ] Require synchronization-based tests and injectable readiness/lifecycle seams; fixed sleeps are not an acceptable correctness strategy.

## Non-goals

- Treating a merely spawned process as healthy without an explicit contract decision.
- Retroactively changing terminal generation history.
- Hot-reload behavior inside the service process.
- Provider-specific health integrations.
- Changing finite-job timeout semantics.

## Related work

- TASK-0035 introduced managed service jobs.
- TASK-0133 documented current generation-owned service behavior and its init-only limitation.
