---
id: TASK-0051
title: Define the historical run duration estimate contract
status: done
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

- [x] Contract defines sample eligibility for passed, failed, cancelled, superseded, timed-out, queued, and debounced time.
- [x] Defines target wall duration as timeout basis; parallel task durations are not summed.
- [x] Locks retention window, median, nearest-rank p90, confidence thresholds, configured floor, safety margin, absolute cap, and overflow behavior.
- [x] Defines zero/insufficient-history fallback and optional configured timeout hint semantics.
- [x] Defines stable execution-signature inputs and secret-safe persisted metadata.
- [x] Defines XDG state location, workspace identity, schema version, corruption recovery, permissions, writer model, and size limits.
- [x] Defines additive protocol fields, capability flag, snapshot-at-run-start behavior, and legacy client/server fallback.
- [x] Explicit timeout override precedence and progress vocabulary are documented; estimate never changes freshness or implies stuck/remaining time.
- [x] Rust and pi-watcher golden fixture matrix is written before implementation.

## Completed

Delivered `docs/RUN-DURATION-ESTIMATES-CONTRACT.md` (normative, §1-§9):
sample eligibility table, estimator formulas (median/p90 nearest-rank, retention 20, floor, margin `p90*1.5+2s`, cap 15m), zero-history fallback + `timeout_hint` config, SHA-256 execution signature + XDG v1 persistence (0600, atomic, quarantine), additive protocol fields (`durationEstimates` capability, `targets`/snapshot `estimate`), timeout precedence + progress vocabulary, and the pre-implementation golden fixture matrix (estimator/signature/persistence/wire).

## Notes

See `.tmp/reports/13-04-26/historical-run-duration-estimates.md` and `run-estimate-implementation-blueprint.md`.

