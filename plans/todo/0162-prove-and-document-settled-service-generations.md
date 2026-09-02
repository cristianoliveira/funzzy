---
id: TASK-0162
title: Prove and document settled service generations
status: todo
depends_on: [TASK-0161]
priority: high
tags: [integration-tests, docs, services, readiness, control-socket, pi-watcher, reliability]
---

# Prove and document settled service generations

## Problem

Users and agent clients need black-box proof that a healthy background service no longer makes a completed pipeline look perpetually active, while service failures and shutdown remain visible and safe.

## Acceptance criteria

- [ ] Add a spawned-watcher test where finite checks pass, a managed service reaches readiness, the exact generation reports terminal success, and the service remains alive.
- [ ] Prove a service that exits or fails its readiness contract before settlement fails the generation and leaves attributable evidence.
- [ ] Prove a post-settlement service failure is reported through the approved service-health surface without rewriting the terminal generation result.
- [ ] Prove later unrelated and service-selecting generations follow TASK-0160 ownership/replacement semantics without leaking or duplicating processes.
- [ ] Prove valid reload, invalid reload, exact cancellation, supersession, SIGINT/SIGTERM, and forced termination reap the correct service process groups.
- [ ] Prove local human output, control status/await/events/output, and pi-watcher rendering agree that generation outcome and service health are distinct.
- [ ] Prove success/failure hooks run once at the approved generation boundary and post-settlement service events cannot produce contradictory hook history.
- [ ] Update README, usage, advanced guidance, canonical schema/help/examples, and `SERVICE-LIFECYCLE-CONTRACT.md` to replace the old “live service means running generation” model.
- [ ] Document the exact readiness guarantee and warn against interpreting weaker readiness as application health.
- [ ] Run focused Rust tests, integration gates, documentation/config drift gates, and pi-watcher checks through configured watcher targets.

## Test constraints

Use explicit readiness barriers and bounded harness deadlines. Do not use narrow sleeps as correctness assertions.
