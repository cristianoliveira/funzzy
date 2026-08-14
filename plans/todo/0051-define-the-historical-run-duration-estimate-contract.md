---
id: TASK-0051
title: Define the historical run duration estimate contract
status: todo
depends_on: [TASK-0042]
priority: high
tags: [design, duration, estimates, agents, determinism]
---

# Define the historical run duration estimate contract

## Problem
Agents need bounded timeout hints from observed runs, but duration samples, confidence, invalidation, persistence, and fallback semantics are not yet a compatibility contract.

## Context

Start with exact configured target wall time because it directly supports `watcher_verify`. Task-level prediction and automatic filesystem-plan estimates are later extensions.

## Acceptance criteria

- [ ] Contract defines sample eligibility for passed, failed, cancelled, superseded, timed-out, queued, and debounced time.
- [ ] Defines target wall duration as timeout basis; parallel task durations are not summed.
- [ ] Locks retention window, median, nearest-rank p90, confidence thresholds, configured floor, safety margin, absolute cap, and overflow behavior.
- [ ] Defines zero/insufficient-history fallback and optional configured timeout hint semantics.
- [ ] Defines stable execution-signature inputs and secret-safe persisted metadata.
- [ ] Defines XDG state location, workspace identity, schema version, corruption recovery, permissions, writer model, and size limits.
- [ ] Defines additive protocol fields, capability flag, snapshot-at-run-start behavior, and legacy client/server fallback.
- [ ] Explicit timeout override precedence and progress vocabulary are documented; estimate never changes freshness or implies stuck/remaining time.
- [ ] Rust and pi-watcher golden fixture matrix is written before implementation.

## Notes

See `.tmp/reports/13-04-26/historical-run-duration-estimates.md` and `run-estimate-implementation-blueprint.md`.

