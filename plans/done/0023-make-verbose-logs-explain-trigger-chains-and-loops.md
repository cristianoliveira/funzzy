---
id: TASK-0023
title: Make verbose logs explain trigger chains and loops
status: done
depends_on: [TASK-0015, TASK-0018, TASK-0022, TASK-0043]
priority: high
tags: [rust, cli, logging, diagnostics, determinism, tdd]
---

# Make verbose logs explain trigger chains and loops

## Problem
Current verbose output prints separators and raw Debug event dumps such as `Events Ok(...)`, but does not show exact path and rule that triggered each task. When a command writes a watched file and creates a feedback loop, user sees repeated execution without a causal chain or actionable hint.

## Context

Replace ad-hoc `stdout::verbose(&format!(...))` calls with small typed diagnostic events rendered consistently to terminal and `--log-file`. Optimize for answering:

1. Which event batch and exact path started this evaluation?
2. How was path normalized and which event kind was observed?
3. Which task matched which effective `change` rule?
4. Did an `ignore` rule win, including inherited/group rules?
5. What run generation and commands were scheduled?
6. Did another watched path appear while or shortly after that run?
7. Is same task/path chain repeating enough to suggest feedback loop?
8. How did run finish, fail, cancel, or get superseded?

Filesystem notifications cannot prove that child command caused later write. Diagnostics must say `observed during/after run` and `possible feedback loop`, never claim causation. Use deterministic event/batch sequence and generation for correlation. If loop heuristic uses time window, inject monotonic clock so tests remain deterministic.

## Acceptance criteria

- [ ] Snapshot/behavior tests first cover startup, native event batch, IPC emit, ignored path, unmatched path, selected tasks, success, failure, cancellation, config reload, malformed watcher event, repeated self-trigger candidate, and unrelated rapid events that must not warn.
- [ ] `--verbose` emits one concise lifecycle record per decision instead of separator banners and duplicate raw/formatted event dumps.
- [ ] Records use stable vocabulary and fields for source (`init`, `filesystem`, `control`, `config`), batch/event sequence, event kind, path, normalized path, task, effective rule origin, decision, generation, command, outcome, duration, and cancellation reason where applicable.
- [ ] Every scheduled run can be traced from event batch and exact changed path through matched task/rule and command outcome using sequence/generation identity.
- [ ] For grouped config, diagnostics distinguish task-local from inherited `change`/`ignore` rule responsible for decision.
- [ ] Ignored and unmatched paths explain reason without executing work.
- [ ] Repeated trigger chains emit bounded `possible feedback loop` warning naming task, repeated path/rule, repeat count, related generation(s), and suggested ignore pattern direction.
- [ ] Loop warning is heuristic and never states child command definitively caused filesystem event.
- [ ] Loop detection cannot alter scheduling, cancellation, or task results; diagnostics are observational only.
- [ ] Loop heuristic state is bounded and deterministic, with injected time/threshold policy where needed.
- [ ] Startup diagnostics show config path, workspace root, watch roots, task count, busy policy, run-on-init state, log destination, and control socket without dumping full config repeatedly.
- [ ] Text ordering and field labels are deterministic; tests do not depend on real timestamps or nondeterministic map/debug formatting.
- [ ] Terminal and log file contain same diagnostic semantics; log file strips ANSI and does not duplicate records.
- [ ] Secrets are not added to diagnostics: environment values and raw JSON-RPC payloads are not logged; command rendering follows documented existing exposure policy.
- [ ] Normal mode output remains unchanged except improvements explicitly accepted by TASK-0014.
- [ ] Existing verbose integration tests assert user-facing decisions rather than internal `Debug` representation such as `Events Ok(...)`.

## Example target output

```text
Funzzy debug: batch=17 source=filesystem kind=modify path=generated/api.rs normalized=generated/api.rs
Funzzy debug: batch=17 decision=matched task="generate API" change="**/*.rs" rule_origin=task
Funzzy debug: batch=17 run=42 policy=restart commands=1
Funzzy debug: run=42 command=1/1 state=started command="make generate"
Funzzy debug: batch=18 source=filesystem kind=modify path=generated/api.rs observed_after_run=42
Funzzy warning: possible feedback loop task="generate API" path=generated/api.rs change="**/*.rs" repeats=3 hint="consider ignoring generated/**"
Funzzy debug: run=42 state=passed duration=0.842s
```

Exact syntax is decided in TASK-0014; semantics above are required.

