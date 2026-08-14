---
id: TASK-0071
title: Define the sequential debugging override contract
status: done
depends_on: [TASK-0024, TASK-0042]
priority: high
tags: [design, concurrency, debugging, control-socket, agents, determinism]
---

# Define the sequential debugging override contract

## Problem
When a parallel target fails nondeterministically, users and agents need an exact comparison run with scheduler concurrency disabled, but config edits would reload watcher, alter freshness, and contaminate diagnosis.

## Context

Canonical surface is `--sequential`, not a general jobs tuner. It is equivalent to effective task concurrency one and names diagnostic intent. It must not mutate `.watch.yaml` or automatically rerun side-effecting commands.

## Acceptance criteria

- [ ] Contract defines local one-shot, watch-session, and exact control-generation scope for `--sequential`.
- [ ] Only scheduler concurrency changes; selection, stage barriers, commands, cwd/env, templates, fail-fast, process ownership, streams, and timeout stay identical.
- [ ] Precedence is explicit sequential override → effective 1 over configured concurrency; later native generations remain configured unless watcher process started sequentially.
- [ ] Correlated snapshot/result reports configured concurrency, effective concurrency, and source without changing existing identity/freshness semantics.
- [ ] Execution signature includes effective concurrency so sequential duration history cannot contaminate parallel estimate.
- [ ] Capability advertises exact-generation sequential override and legacy server rejection/fallback is explicit, never silent parallel execution.
- [ ] Exit codes cover accepted, unsupported, malformed/conflicting, superseded, cancelled, failed, and timeout paths.
- [ ] Agent diagnosis says `parallel-sensitive` only after comparable parallel failure/sequential pass; it never claims proven race or stuck process.
- [ ] Out of scope names races internal to one command/service/filesystem/external dependency and generic `--jobs N` tuning.

## Notes

