---
id: TASK-0152
title: Define settled failure hook contract
status: done
depends_on: []
priority: high
tags: [design, config, hooks, watcher, agents, determinism]
---

# Define settled failure hook contract

## Problem
A terminal failure does not prove the workspace has settled, so an immediate custom failure hook can send an LLM agent stale work while another generation is about to replace it.

## Desired outcome

Users can keep `hooks.failure` as a generic finite custom command while explicitly requiring a failed outcome to remain current for a bounded settle period before the command runs.

## Proposed configuration

```yaml
hooks:
  failure:
    run: ./scripts/tell-my-agent
    settle: 30s
```

The existing string form remains the immediate shorthand:

```yaml
hooks:
  failure: ./scripts/on-failure
```

## Acceptance criteria

- [x] Define `settle` as a positive bounded duration during which the failed generation must remain the latest outcome.
- [x] Define when the settle clock starts and what counts as newer work.
- [x] Any newer accepted generation cancels the pending settled hook; a newer failure starts its own settle period and a newer pass leaves no failure hook pending.
- [x] The settle wait does not block scheduling, starting, or publishing the outcome of a newer generation.
- [x] Define deterministic ordering when settle expiry races a new generation, including cancellation and reaping when the custom command has started.
- [x] Preserve immutable generation correlation: the pending command and configuration revision belong to the generation that failed.
- [x] Define finite `run`, watched run, control await, reload, cancellation, supersession, shutdown, and hook-failure behavior.
- [x] Preserve current scalar `hooks.failure` behavior byte-for-behavior and reject malformed objects with actionable errors.
- [x] State whether the object form is failure-only or shared by success hooks; avoid accidental scope expansion.

## Non-goals

- Detect whether an LLM agent is busy or idle.
- Add a notification provider or platform-specific integration.
- Guarantee that an external side effect can be recalled after the custom command starts.
- Change workflow success/failure based on hook outcome.

## Notes

Related contract: `docs/RUN-HOOKS-CONTRACT.md`. Related completed task: TASK-0040.

