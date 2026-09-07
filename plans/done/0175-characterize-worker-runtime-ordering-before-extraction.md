---
id: TASK-0175
title: Characterize worker runtime ordering before extraction
status: done
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

- [x] A test-to-invariant inventory identifies the baseline and any remaining coverage gaps.
- [x] Normal completion and failure/cancellation outcomes have observable assertions; assertions do not merely inspect private field layout.
- [x] Accepted cancellation/shutdown precedes child facts; stale requests do not affect newer generations.
- [x] Existing tests protect reaping before service replacement, frozen revisions, and terminal event ordering.
- [x] Any added test demonstrates its missing behavior before a related logic correction; if no gap exists, document the evidence instead of inventing a failing test.
- [x] No fixed sleeps are added as assertion strategy; spawned tests reuse bounded polling and cleanup helpers.

## Verification

Run focused worker and domain-boundary tests. If mode-parity cases change, run `tests/executor_contract.rs` with integration behavior enabled. Use the active watcher for final verification; default tests alone do not prove process/filesystem behavior. Record test names and results, not an unmeasured coverage percentage.

## Evidence

- Inventory: `.tmp/reports/04-09-26/task-0175-ordering-inventory.md` maps every ordering invariant to its protecting test and names the two gaps closed here.
- Observable assertions: all coverage asserts through public `WorkerEvent` streams and CLI/control surfaces; no private-field inspection.
- Cancellation precedence: `worker_cycle_resolves_commands_before_child_observations` (commands before child facts), `stale_cancel_does_not_affect_a_newer_generation` (stale no-op), `explicit_cancel_terminates_the_active_run` (never-Finished check).
- Protected invariants (existing, named in inventory): reaping-before-replacement authorization (`worker_reservation_requires_reaped_matching_handoff_before_start`, `worker_reload_replacement_authorizes_start_only_after_reap`, `worker_cancellation_after_reap_suppresses_replacement_start`), frozen revisions (`schedule_with_explicit_revision_freezes_that_revision_on_the_generation`, `manual_generation_keeps_frozen_revision_across_reload_commit`, `worker_pool_rejects_stale_reservation_after_newer_reload`), terminal ordering (`generation_ids_are_never_reused_after_terminal`, burst/newest-wins, discarded-queued terminal, settlement priority).
- Gap-driven additions only: `d38c990` adds exactly the two missing orderings — `normal_generation_orders_started_terminals_and_finished_strictly` and `supersession_terminal_precedes_successor_start_and_predecessor_never_finishes` — each demonstrated absent from the inventory first; no logic correction accompanied them.
- Determinism: serial barriers fix completion order; existing bounded polling helpers (`expect_event`, `collect_until_finished`, `wait_until`) reused; no new sleeps.
- Results: `cargo test --lib workers::` 56 passed (incl. 2 new); `cargo test --test domain_boundaries` 8 passed; watcher gen160 full unit gate passed fresh. `tests/executor_contract.rs` not rerun: no mode-parity change (tests-only slice).
- Riskiest ordering for TASK-0176: cross-generation terminal-before-successor (predecessor `Cancelled(superseded_by)` strictly before successor `Started`, predecessor never `Finished`).


## Likely files

`src/workers.rs` colocated tests; `tests/executor_contract.rs` only for a demonstrated missing mode-parity case.

## Non-goals

No runtime extraction, new scheduler trait, protocol changes, or constructor API changes.
