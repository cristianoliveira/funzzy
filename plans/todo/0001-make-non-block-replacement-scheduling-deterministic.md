---
id: TASK-0001
title: Make non-block replacement scheduling deterministic
status: doing
depends_on: []
priority: high
tags: [rust, worker, reliability]
---

# Make non-block replacement scheduling deterministic

## Problem

Cancellation and scheduling use separate channels. During rapid A/B/C changes, queued cancellation and work messages can interleave, allowing a superseded run to start. Non-block mode should deterministically execute newest requested generation.

## Scope

- `src/workers.rs`
- `src/cli/watch_non_block.rs`
- Focused unit and integration tests

## Acceptance criteria

- [ ] A failing deterministic test demonstrates burst replacement without timing sleeps.
- [ ] Scheduling and replacement are represented atomically, preferably through one worker command stream.
- [ ] Older queued generations are discarded before process spawn.
- [ ] Active child receives graceful termination when replaced.
- [ ] Latest generation starts and reports correct `RunEvent`/control state.
- [ ] Existing non-block and control-socket behavior remains compatible.

## Verification

- Focused worker tests cover happy and unhappy paths.
- Feature-gated non-block and control-socket integration tests pass.

