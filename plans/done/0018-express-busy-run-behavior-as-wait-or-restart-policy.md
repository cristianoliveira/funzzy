---
id: TASK-0018
title: Express busy-run behavior as wait or restart policy
status: done
depends_on: [TASK-0015]
priority: normal
tags: [rust, cli, process, worker, tdd]
---

# Express busy-run behavior as wait or restart policy

## Problem
The --non-block name exposes implementation rather than telling users what happens when a change arrives during an active run.

## Context

Map user-facing policy to existing blocking and non-blocking strategies without duplicating watch orchestration. Preserve control-socket implication explicitly.

## Acceptance criteria

- [x] Tests first cover default wait, explicit wait, restart cancellation, environment override, and control-socket implication.
- [x] CLI exposes policy vocabulary decided in TASK-0014, with a convenient restart form if specified.
- [x] Wait completes active run before processing replacement work. (unchanged blocking executor)
- [x] Restart cancels active child and schedules newest generation deterministically. (unchanged non-block executor; on_busy=Restart maps to it)
- [x] Invalid policy values fail during parsing with actionable choices.
- [x] Old `--non-block` path is removed or retained only if TASK-0014 explicitly requires migration compatibility. (removed; FUNZZY_NON_BLOCK still maps to restart)

Deliverable: commit `d3a22cd`.

## Notes

