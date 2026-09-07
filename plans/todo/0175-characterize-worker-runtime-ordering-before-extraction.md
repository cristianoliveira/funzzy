---
id: TASK-0175
title: Characterize worker runtime ordering before extraction
status: doing
depends_on: []
priority: high
tags: [rust, architecture, worker, tests]
---

# Characterize worker runtime ordering before extraction

## Problem
The worker consumer loop mixes command handling with child observations. Extracting it without a focused observable baseline can change cancellation, service ownership, or generation ordering.

## Context

Start the worker refactor here. `src/workers.rs::with_root_and_concurrency_and_outputs_and_approval` embeds the consumer loop. Existing tests already cover many transitions; inventory them before adding cases. TASK-0171 extracted pure lifecycle decisions, but did not isolate this runtime loop.

## Scope and approach

- Inspect colocated worker tests for commands-before-child-observation, exact/stale cancellation, supersession, settlement hooks, frozen revisions, and service replacement authorization.
- Record which tests protect each invariant. Add only missing observable cases; do not duplicate covered behavior to satisfy a test-count target.
- Use deterministic scheduler facts and existing fake clock/process seams where available. Keep real-process behavior in the established integration harness.
- Leave production orchestration unchanged in this task. Preserve useful tests for the next two extraction tasks.

## Acceptance criteria

- [ ] A test-to-invariant inventory identifies the baseline and any remaining coverage gaps.
- [ ] Normal completion and failure/cancellation outcomes have observable assertions; assertions do not merely inspect private field layout.
- [ ] Accepted cancellation/shutdown precedes child facts; stale requests do not affect newer generations.
- [ ] Existing tests protect reaping before service replacement, frozen revisions, and terminal event ordering.
- [ ] Any added test demonstrates its missing behavior before a related logic correction; if no gap exists, document the evidence instead of inventing a failing test.
- [ ] No fixed sleeps are added as assertion strategy; spawned tests reuse bounded polling and cleanup helpers.

## Verification

Run focused worker and domain-boundary tests. If mode-parity cases change, run `tests/executor_contract.rs` with integration behavior enabled. Use the active watcher for final verification; default tests alone do not prove process/filesystem behavior. Record test names and results, not an unmeasured coverage percentage.

## Likely files

`src/workers.rs` colocated tests; `tests/executor_contract.rs` only for a demonstrated missing mode-parity case.

## Non-goals

No runtime extraction, new scheduler trait, protocol changes, or constructor API changes.
