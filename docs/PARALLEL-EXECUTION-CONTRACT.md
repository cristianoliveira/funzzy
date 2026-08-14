# Funzzy Parallel Execution Contract

> Status: **draft** — defined by TASK-0024. Drives TASK-0025 through TASK-0030.
> This workstream is independent of the V2 CLI redesign (TASK-0020) and does
> not block it. Existing configurations without `parallel` keep today's
> sequential behavior exactly.

## 1. Task model

- A **task** is the concurrency unit. It has a stable identity: name plus
  position in the parsed workflow.
- A **named `parallel` group** is an opt-in declaration that consecutive
  tasks may run concurrently. Groups are never inferred from independence.
- **Commands inside one task remain strictly sequential**, in declared order.
- A task without `parallel` is **serial**: it runs alone between barriers.

### Configuration shape

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

Executes as `prepare -> [checks: lint || test] -> package`.

### Group occurrence identity

A group is identified by **name plus contiguous position**, not by name alone:

- `A@checks, B@checks` → one occurrence `[A || B]`.
- `A@checks, B, C@checks` → `[A] -> B -> [C]`: the second `checks` reuse
  starts a **new barrier** and never reconnects to the first.
- Serial task, changed group name, or end of list closes the current group
  and creates a barrier.

## 2. Topology and barriers

- Only **consecutive** tasks sharing the same non-empty group name run
  together.
- The next task or group occurrence starts only after **all** selected
  members of the current occurrence reach a terminal outcome.
- A barrier is a scheduling fence, not an execution order: inside a group,
  start/completion order is unspecified.
- Reusing a group name after a barrier never merges the two occurrences.

### Filtering preserves topology

Path/target/init filtering removes unmatched tasks **without merging
originally separate group occurrences**:

- `A@x, B, C@x` where `B` is filtered out → `[A] -> [C]`; the two `x`
  occurrences stay separate barriers even though no serial task remains
  between them.
- A group with only one selected member executes that member normally.
- Empty stages disappear safely; the plan keeps valid barrier order.

## 3. Concurrency bound

- `on.concurrency` is an optional global cap on simultaneously active tasks.
- Default is the injected available-parallelism provider
  (`std::thread::available_parallelism`), resolved once at plan time.
- Accepted values: positive integers. `0`, negative, non-integer, or
  non-numeric values fail configuration validation with an actionable
  task-local error.
- Effective concurrency for one group occurrence is
  `min(on.concurrency, selected members in occurrence)`.
- `concurrency: 1` is valid and means tasks run sequentially inside barrier.
- CLI override is **deferred** unless the V2 CLI contract explicitly adds it.
- Concurrency is a bounded resource policy (worker pool / semaphore), never
  `thread::spawn` per task.

## 4. Outcomes

- The overall run outcome derives deterministically from per-task outcomes.
- **Task outcome** is one of: passed, failed, cancelled, skipped.
- **Run outcome** is one of: passed, failed, cancelled (restart replacement).
- With **fail-fast off** (default), every task selected for the run continues
  even after failures — matching today's behavior — and the final run is
  failed if any task failed.
- With **fail-fast on**, the first failure:
  - cancels active sibling tasks in the same occurrence,
  - skips queued members of that occurrence,
  - skips all later stages.
- Result combination is **order-independent** and keyed by task identity.
  Inside a group, completion and display order is explicitly not
  contractual; tests compare task-keyed outcomes, never sequence.

## 5. Busy-run policies (wait / restart)

- **Wait** (`--on-busy wait`, default): a change arriving during a run is
  processed after the whole run reaches a terminal outcome (all stages,
  including every group barrier).
- **Restart** (`--on-busy restart` / `--restart`): a change cancels and
  reaps **all** active tasks across every occurrence, discards queued work,
  and starts the newest generation only after every active child and
  forwarding thread is reaped.

## 6. Output

- Live child output is attributed to task (and command when needed) and is
  line-atomic; byte-level interleaving must not corrupt lines.
- Group/task identity is present in final summaries.
- `--log-file` preserves the same attribution without duplicating forwarded
  output.
- Output buffering is bounded or streamed; the implementation cannot
  accumulate unlimited child output in memory.

## 7. Scope and compatibility

- **No task dependency DAG is introduced.** Assigning tasks to the same
  named contiguous group declares independence and nothing more.
- Existing control JSON-RPC fields remain backward compatible. Any per-task
  detail is additive and coordinated with `pi-watcher` decoders/tests.
- Configurations without `parallel` parse and execute exactly as today.
- A configured group may contain one selected task after filtering; it
  executes normally (no degenerate behavior).

## 8. Black-box matrix

| Configuration | Expected execution |
|---|---|
| `A, B, C` | `A -> B -> C` |
| `A, B@checks, C@checks, D` | `A -> [B \|\| C] -> D` |
| `A@one, B@two` | `[A] -> [B]` |
| `A@x, B, C@x` | `[A] -> B -> [C]`; reused name does not reconnect |
| `A@x, B@x` with `concurrency: 1` | both run sequentially within same barrier |
| only `B@x` matches path/target/init filter | `B` runs alone; topology remains valid |
| separator task does not match | groups on either side remain separate |
| member fails, fail-fast off | siblings and later selected tasks finish; run fails |
| member fails, fail-fast on | active siblings cancel; queued/later tasks skip; run fails |
| new event under restart policy | all active members cancel/reap before newest generation starts |

### Test matrix (TASK-0025–0029)

- Limits: `concurrency` 1, 2, greater than task count; spawn failure occupies and
  releases one slot and is reported as task failure.
- Ordering: commands within task sequential; group memberships and barriers
  asserted, never incidental order.
- Outcomes: all pass, one fail, many fail, spawn failure, fail-fast,
  cancelled, superseded generations.
- Cancellation: repeated cancel/replacement leaves no child, process group,
  forwarding thread, or executor thread behind.
- Output: interleaved stdout/stderr, partial lines, non-UTF8 policy,
  bounded buffering.
- Shutdown: Ctrl-C, config reload, worker drop, fail-fast, and normal
  shutdown use one ownership path.

## 9. Performance success criterion

- Primary proof is **deterministic overlap**: fake process runner and
  barriers prove concurrent tasks overlap and serial tasks do not, without
  sleeps.
- Supporting benchmark: independent-task batch latency approaches the slowest
  task rather than the sum, with generous bounds and environment recorded.
- No claim that parallelism makes every workload faster; CPU/process cost
  and recommended concurrency guidance are documented.
