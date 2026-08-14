---
id: TASK-0055
title: Expose duration estimates through control capabilities and snapshots
status: todo
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

- [ ] Golden tests first cover estimate absent, configured fallback, measured low/medium/high confidence, bounds, and legacy field compatibility.
- [ ] `targets` adds optional estimate with typical, upper, recommended timeout, samples, confidence, and source.
- [ ] Target response retrieves estimate dynamically rather than freezing it when server starts.
- [ ] Correlated running/terminal snapshot carries estimate selected at start for exact target generation.
- [ ] `capabilities.features.durationEstimates` is true only when history/estimate surface is active and limits are declared.
- [ ] Existing clients ignoring fields continue to decode status, targets, run, await, and subscription.
- [ ] Human and structured control rendering use same domain estimate and deterministic duration format.
- [ ] Protocol never exposes execution signature inputs, environment values, or state-file path.
- [ ] Rust fixtures are synchronized with pi-watcher decoders before completion.

## Notes

