---
id: TASK-0127
title: Prove and document current-run job duration reports
status: done
depends_on: [TASK-0125, TASK-0126]
priority: high
tags: [integration-tests, docs, duration, cli, control-socket, reliability]
---

# Prove and document current-run job duration reports

## Problem
Unit-level timing and formatting changes do not prove real Funzzy executions report truthful per-job elapsed time consistently across local, watched, parallel, cancelled, recovered, and control-driven runs.

## Context

Use deterministic/fake-clock seams for exact values and black-box assertions for shape/cross-surface parity. Avoid host-speed thresholds that make CI flaky.

## Acceptance criteria

- [x] Add black-box proof that local `fzz run` reports every job's final state and duration plus separate generation total.
- [x] Prove serial and parallel jobs use identical per-job semantics and preserve declaration order regardless of completion order.
- [x] Prove started cancellation has partial duration while skipped/never-started jobs have no fabricated duration.
- [x] Prove recovered job appears once with final state and duration through final verification.
- [x] Prove local human output, control human output, structured snapshots, and `task_terminal` events agree on job identity, state, and measured milliseconds.
- [x] Prove both binary aliases (`funzzy` and `fzz`) and blocking/restart strategies use same reporting contract.
- [x] Keep timing tests deterministic through fake clocks or bounded structural assertions; never require one real command to finish within fragile host-time threshold.
- [x] Verify existing generation duration estimates and persisted target history remain unchanged.
- [x] Update README/USAGE/control guidance with one concise current-run report example and define absent/cancelled/recovery semantics.
- [x] Document job durations are valid independently of parallelism and generation total is measured separately.
- [x] Run focused unit/integration watcher gates and capture evidence for every acceptance criterion.

## Outcome

Black-box proofs now cover local rows and NDJSON terminal records for both aliases, declaration order despite parallel completion, control human/structured/event parity, started cancellation, skipped `null`, recovery finalization, and existing duration-history behavior. The filepath-template fixtures now assert their stable expansion semantics instead of a byte-for-byte result boundary invalidated by additive job rows. README, usage, and control-socket guidance define parallel independence, separate generation elapsed, and absent/cancelled/recovery behavior.

## Notes

Historical job statistics, command timing, and recovery-phase breakdown are separate validated follow-ups.
