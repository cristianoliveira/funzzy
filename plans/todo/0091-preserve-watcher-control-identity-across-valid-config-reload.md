---
id: TASK-0091
title: Preserve watcher control identity across valid config reload
status: todo
depends_on: [TASK-0089, TASK-0090, TASK-0050, TASK-0082]
priority: high
tags: [rust, watcher, control-socket, identity, reload, snapshots]
---

# Preserve watcher control identity across valid config reload

## Problem
The watcher control server currently treats config edit as instance termination, resetting generation identity, subscriptions, retained output, and agent freshness despite process configuration remaining valid.

## Context

Server remains Funzzy-owned. This task changes watcher protocol truth; pi-watcher integration is out of scope for this plan.

## Acceptance criteria

- [ ] Valid reload preserves instance token/start time and monotonic batch/generation sequence; tests remove old assumption that config reload always changes instance.
- [ ] Snapshot/status/await/subscription/run/output/cancel expose frozen config revision for correlated generation additively.
- [ ] Lifecycle emits bounded `configReloading`, `configReloaded`, or terminal `configInvalid` from same state source; formatting-only no-op is explicit/quiet per contract.
- [ ] Active await/subscription connection survives valid reload and receives revision transition without disconnect/reconnect.
- [ ] Retained outputs and exact output references from prior revisions remain retrievable under same instance until ordinary eviction.
- [ ] Target/capability responses after commit reflect new jobs/estimates/socket facts consistently, never mixed revision.
- [ ] Synthetic emit/control run concurrent with reload bind to one revision deterministically; stale target has actionable typed outcome.
- [ ] Invalid candidate publishes terminal config diagnostic when possible, cancels/reaps work, closes socket, and process exits nonzero; clients observe disconnect only after terminal event attempt.
- [ ] Restart from external signal/binary replacement still changes instance token; config revision does not weaken true restart freshness.

## Notes
