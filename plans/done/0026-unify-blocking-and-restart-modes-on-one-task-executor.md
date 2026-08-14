---
id: TASK-0026
title: Unify blocking and restart modes on one task executor
status: done
depends_on: [TASK-0025, TASK-0018]
priority: high
tags: [rust, execution, architecture, worker, tdd]
---

# Unify blocking and restart modes on one task executor

## Problem
BlockingStrategy and Worker currently implement separate command loops; adding concurrency to both would duplicate scheduling, failure, cancellation, and result behavior.

## Context

TASK-0006 unified watch orchestration. This task unifies lower execution engine currently duplicated by `BlockingStrategy::execute_tasks` and `workers::ActiveRun`.

## Acceptance criteria

- [x] Contract tests first prove same plan produces equivalent outcomes in wait and restart modes with concurrency one.
- [x] One executor owns process spawn, polling/waiting, fail-fast, cancellation, and outcome collection.
- [x] Blocking and restart strategies only decide busy-run policy and submit/cancel plans; they do not implement command loops.
- [x] Executor receives process runner, clock, concurrency limit, and event sink explicitly.
- [x] Existing generation monotonicity and newest-run replacement guarantees remain intact.
- [x] Control state and verbose diagnostics consume executor events rather than worker internals.
- [x] Shutdown joins executor resources and reaps child processes.

## Notes

Completed with one injected executor shared by wait and restart policies. Executor lifecycle events now drive control state and verbose tick diagnostics. Fresh Funzzy watcher generation 56 passed build, lint, unit, integration, and Nix checks.

