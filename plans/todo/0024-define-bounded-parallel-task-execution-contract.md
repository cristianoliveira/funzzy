---
id: TASK-0024
title: Define bounded parallel task execution contract
status: todo
depends_on: [TASK-0014]
priority: high
tags: [rust, concurrency, execution, design, performance]
---

# Define bounded parallel task execution contract

## Problem
Funzzy currently flattens selected tasks into one sequential command list, so parallel execution would otherwise make task ordering, fail-fast, output, cancellation, and compatibility ambiguous.

## Context

Parallelize only tasks user assigns to an explicit named group; do not infer independence. Commands within each task remain sequential. Consecutive tasks sharing a `parallel` group run together. A serial task, a different group name, or end of list closes group and creates barrier. Treat concurrency as bounded resource policy, not `thread::spawn` per task.

Selected syntax:

```yaml
tasks:
  - name: prepare
    run: make prepare

  - name: lint
    parallel: checks
    run: make lint

  - name: test
    parallel: checks
    run: make test

  - name: package
    run: make package
```

This means `prepare -> [checks: lint || test] -> package`. Reusing `checks` later does not reconnect separated tasks; group identity is name plus contiguous position.

This is later workstream and does not block TASK-0020 CLI redesign release.

## Acceptance criteria

- [ ] Contract defines task as concurrency unit, named `parallel` group as opt-in, and preserves command order within task.
- [ ] Existing tasks without `parallel` remain sequential by default; no task becomes concurrent merely because independence is guessed.
- [ ] Only consecutive tasks with same non-empty group name run together; separated reuse of name creates distinct group occurrence.
- [ ] Serial task, changed group name, or end of list closes group; next task/stage starts after all selected members terminate.
- [ ] Filtering occurs without losing original topology: unmatched tasks are skipped but cannot merge originally separate group occurrences.
- [ ] Concurrency configuration, safe default, `jobs=1`, explicit numeric limit, and optional `auto` behavior are decided.
- [ ] Overall run success/failure/cancelled semantics are defined from per-task outcomes.
- [ ] Fail-fast defines what happens to queued tasks and already-running sibling tasks.
- [ ] Wait/restart busy policies define cancellation and replacement across all active tasks.
- [ ] Live output attribution includes group/task identity; completion and display order inside group is explicitly not contractual.
- [ ] Result combination is order-independent and keyed by task identity.
- [ ] No task dependency DAG is introduced initially; assigning tasks to same named contiguous group declares independence.
- [ ] Performance success criterion compares batch latency against sequential sum without relying solely on flaky wall-clock assertions.
- [ ] Test matrix covers limits, ordering, all outcome combinations, cancellation, spawn errors, output, and shutdown.

## Proposed contract decisions

- YAML field is `parallel: <group-name>`; boolean values and empty names are invalid with actionable config errors.
- Group occurrence identity is `(group name, contiguous position)`, so a later reuse starts a new barrier.
- Without fail-fast, every command/task selected for run continues after failures, preserving current behavior; group waits for all members before next barrier and final run remains failed.
- With fail-fast, first failure cancels active siblings, skips queued group members, and skips later work.
- A configured group may contain one selected task after path/target filtering; it executes normally.
- `on.jobs` is optional global cap for simultaneously active tasks. Default is `available_parallelism`; positive integer values are accepted and zero/non-integer values fail config validation. CLI override is deferred unless V2 CLI contract explicitly adds it.
- Result/output order inside group is unspecified; task identity is mandatory.
- Existing control JSON-RPC fields remain compatible. Any per-task detail is additive and coordinated with `pi-watcher`.

## Black-box matrix

| Configuration | Expected execution |
|---|---|
| `A, B, C` | `A -> B -> C` |
| `A, B@checks, C@checks, D` | `A -> [B || C] -> D` |
| `A@one, B@two` | `[A] -> [B]` |
| `A@x, B, C@x` | `[A] -> B -> [C]`; reused name does not reconnect |
| `A@x, B@x` with `jobs: 1` | both run sequentially within same barrier |
| only `B@x` matches path/target/init filter | `B` runs alone; topology remains valid |
| separator task does not match | groups on either side remain separate |
| member fails, fail-fast off | siblings and later selected tasks finish; run fails |
| member fails, fail-fast on | active siblings cancel; queued/later tasks skip; run fails |
| new event under restart policy | all active members cancel/reap before newest generation starts |

## Notes

