---
id: TASK-0091
title: Preserve watcher control identity across valid config reload
status: doing
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

- [x] Valid reload preserves instance token/start time and monotonic batch/generation sequence; tests remove old assumption that config reload always changes instance.
- [x] Snapshot/status/await/subscription/run/output/cancel expose frozen config revision for correlated generation additively.
- [x] Lifecycle emits bounded `configReloading`, `configReloaded`, or terminal `configInvalid` from same state source; formatting-only no-op is explicit/quiet per contract.
- [x] Active await/subscription connection survives valid reload and receives revision transition without disconnect/reconnect.
- [x] Retained outputs and exact output references from prior revisions remain retrievable under same instance until ordinary eviction.
- [x] Target/capability responses after commit reflect new jobs/estimates/socket facts consistently, never mixed revision.
- [x] Synthetic emit/control run concurrent with reload bind to one revision deterministically; stale target has actionable typed outcome.
- [x] Invalid candidate publishes terminal config diagnostic when possible, cancels/reaps work, closes socket, and process exits nonzero; clients observe disconnect only after terminal event attempt.
- [x] Restart from external signal/binary replacement still changes instance token; config revision does not weaken true restart freshness.

## Notes

Design decisions (implemented, all green):

1. **Shared config is the control source.** `NonBlockStrategy` no longer keeps
   a private `Watches` copy; it holds the same `Arc<Mutex<Watches>>` the
   routing loop locks and the reload transaction swaps. `targets` (via a
   request-time `TargetsProvider`), `run`, `emit`, and estimates resolve from
   the shared config under one lock — a reload commit is served by the same
   server with no rebuild, never a mixed revision.
2. **Plan + revision atomic per decision.** `watch_loop` reads the frozen
   revision under the same lock as the plan; `RunStrategy::run_init`/
   `run_change` and the worker `schedule_*` calls carry the explicit
   revision, so a generation concurrent with reload freezes exactly the
   revision it was planned under (AC7). The worker's bound revision remains
   the fallback for legacy/test paths.
3. **One `ConfigLifecycle` state source** (`src/config_lifecycle.rs`):
   bounded transition history; phases serialize exactly as the contract
   events (`configReloading`/`configReloaded`/`configInvalid`). The reload
   thread writes (reloading before prepare, reloaded after commit, invalid
   in `fatal_reload` before shutdown); a formatting-only no-op never
   transitions (quiet). The snapshot broker attaches the source and a
   lifecycle watcher publishes, so subscriptions receive the revision
   transition on the same connection (AC4); `fatal_reload` publishes
   `configInvalid` before the socket closes (AC8).
4. **Additive revision facts across surfaces (AC2).** `run` → `ScheduledRun`
   (runId + revision + revisionHash); `emit` → `EmitOutcome` gains the same;
   `cancel` → `CancelResult::Cancelled` carries the cancelled generation's
   revision (threaded from the worker Run/RunRequest); `output` →
   `OutputRegistry` stores the frozen revision per generation and returns it;
   `status`/`await`/snapshot already carried it via `ControlState`. The CLI
   (`StatusSnapshot`, `ScheduledRunSnapshot`, `EmitSnapshot`,
   `ConfigSnapshot`) decodes the new fields; new `fzz control config`
   subcommand serves the lifecycle.
5. **Typed stale-target outcome (AC7).** `ControlRunError::TargetNotFound`
   maps to RPC `-32016 target_not_found` with `{target, action:
   "reobserve-targets"}` — never a generic message the agent would parse.
6. **Tests.** `tests/control_reload_identity.rs` proves all nine ACs
   black-box (identity preservation, monotonic generations, lifecycle
   subscription continuity, retained output, typed stale target, terminal
   invalid, restart token change). Unit tests cover lifecycle transitions,
   shared-config resolution, explicit-revision freezing, and the wire shapes.

Out of scope (per plan): pi-watcher decoder/fixture updates.
