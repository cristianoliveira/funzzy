---
id: TASK-0055
title: Expose duration estimates through control capabilities and snapshots
status: done
depends_on: [TASK-0047, TASK-0054]
priority: high
tags: [rust, control-socket, protocol, capabilities, duration, tdd]
---

# Expose duration estimates through control capabilities and snapshots

## Problem
Agent clients cannot choose adaptive timeouts unless target discovery and run observations expose optional estimate, confidence, sample count, and recommendation fields compatibly.

## Context

Make target listing calculate current estimate at request time. Capture estimate selected at generation start for stable progress; do not mutate it mid-run as history changes.

## Acceptance criteria

- [x] Golden tests first cover estimate absent, configured fallback, measured low/medium/high confidence, bounds, and legacy field compatibility.
- [x] `targets` adds optional estimate with typical, upper, recommended timeout, samples, confidence, and source.
- [x] Target response retrieves estimate dynamically rather than freezing it when server starts.
- [x] Correlated running/terminal snapshot carries estimate selected at start for exact target generation.
- [x] `capabilities.features.durationEstimates` is true only when history/estimate surface is active and limits are declared.
- [x] Existing clients ignoring fields continue to decode status, targets, run, await, and subscription.
- [x] Human and structured control rendering use same domain estimate and deterministic duration format.
- [x] Protocol never exposes execution signature inputs, environment values, or state-file path.
- [x] Rust fixtures are synchronized with pi-watcher decoders before completion.

## Completed

- `RunEstimate`/`EstimateConfidence`/`EstimateSource` serialize camelCase/lowercase (duration_history.rs).
- `targets_result(targets, provider)` computes each target's estimate at request time via injected `TargetEstimateProvider`; legacy shape unchanged when absent.
- `capabilities_result` gains `durationEstimates` feature + `durationEstimateLimits` (maxSamples/floorMs/capMs) only when a provider is wired; `optionalFields` adds `estimate`.
- `SnapshotBroker::with_estimates` carries `estimate` on the correlated snapshot; `DurationRecorder::estimate_at_start` freezes the estimate at run start (survives terminal, bounded 256).
- `NonBlockStrategy` holds the recorder; `start_control_server` wires the provider from watches + worker concurrency/fail_fast. Composition root (watch_non_block) creates recorder + estimate lookup.
- control_client decodes `durationEstimates`, estimate limits, and target estimates; CLI renders deterministic `format_duration` + `render_estimate` (same domain object as JSON).
- Golden tests: capability flags, targets absent/present estimate, camelCase wire, no signature/env/state-path leakage, frozen snapshot estimate, legacy snapshot no-key.
- pi-watcher decoders ignore the additive fields; fixture/decoder sync lands with pi-watcher TASK-0017 (its board).

## Notes

