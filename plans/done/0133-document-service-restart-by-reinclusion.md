---
id: TASK-0133
title: Document service restart-by-reinclusion model and the init-only service footgun
status: todo
depends_on: []
priority: low
tags: [rust, docs, services, config]
---

# Document service restart-by-reinclusion model and the init-only service footgun

## Problem

Users cannot predict a managed service's lifetime from current documentation. A `service: true` job belongs to the active generation: supersession stops it, and the replacement generation restarts it only when its effective change patterns include it. Consequently, an init-only service dies on the first later generation and stays stopped.

This behavior was uncovered while exploring external-system observation, but managed services are not the chosen primitive for that use case. A blocking finite job has different terminal semantics and is tracked separately by TASK-0134 through TASK-0137.

## Evidence

- Services are reaped when a generation is replaced.
- Non-zero service exits are retried within the existing bounded policy.
- Zero exit is treated as deliberate service stop.
- A service declared only with `run_on_init: true` is not re-included by later path-selected generations.
- Reproduction and source trace: `.tmp/reports/26-08-26/gh-actions-watch-design-a.md`.

## Acceptance criteria

- [ ] Document that services are generation-owned and restarted by re-inclusion, not watcher-owned indefinitely.
- [ ] Document how root and per-job change patterns determine whether a service joins a replacement generation.
- [ ] State that init-only services stop after the first superseding generation and do not silently return.
- [ ] Make `fzz check` reject or actionably warn about `service: true` with `run_on_init` but no change trigger; record which behavior is chosen and why.
- [ ] Document that a generation containing a live service remains running until shutdown, supersession, or terminal service failure.
- [ ] Keep existing restart, cancellation, reload, and legacy configuration behavior unchanged.
- [ ] Add focused validation/documentation tests so canonical config help cannot drift from the lifecycle description.

## Non-goals

- External-provider integrations.
- Treating service exit as a finite observation result.
- Adding service-exit-to-target bindings.
- Changing services to watcher-owned processes.
- Blocking integration-agnostic finite-job observation work.
