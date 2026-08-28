---
id: TASK-0148
title: Clarify exclusive watcher observation feedback
status: todo
depends_on: []
priority: high
tags: [pi-watcher, axi, observation, ux, determinism, tdd]
---

# Clarify exclusive watcher observation feedback

## Problem

`watcher_observe` renders a terminal generation excluded by `afterGeneration` as ambiguous `WAITING gen=N` while the footer correctly reports `failed #N`, causing humans and agents to believe the watcher is still running or contradictory.

Reproduction:

```text
watcher_observe: WAITING gen=16 waitingForGeneration>16 freshness=current (polled)
footer:          watcher: failed #16 7933ms (polled)
```

The wait is valid and exclusive; generation 16 is terminal but intentionally excluded. Feedback must make this obvious without changing behavior.

## Output contract

Excluded baseline:

```text
WAITING gen>16 current=16 state=failed excluded=true freshness=current (polled) waited=477s
```

Newer selected generation:

```text
RUNNING gen=17 selectedAfter=16 freshness=current (polled) waited=2s
```

Timeout before anything newer:

```text
TIMEOUT gen>16 current=16 state=failed excluded=true freshness=current waited=600s (polled)
next: watcher_observe wait=true generation=16
```

## Acceptance criteria

- [ ] Write failing domain tests first for failed/passed/cancelled/running/idle baselines at or below `afterGeneration`, including selector greater than current.
- [ ] Excluded progress states selector threshold, current generation, current state, and `excluded=true`; remove ambiguous `WAITING gen=current` phrasing.
- [ ] A generation greater than selector is labeled with its actual state and `selectedAfter=N`.
- [ ] Timeout before a newer generation carries the same selector/current context plus one copyable exact-generation next action; idle/no-generation timeout gives a matching-trigger hint instead.
- [ ] Heartbeats stay rate-bounded and do not repeat next-action hints; polled/subscription/freshness/waited labels remain accurate.
- [ ] Exact-generation and selector-free progress/result text remain byte-compatible.
- [ ] Exclusive application semantics remain unchanged: terminal baseline ignored, first newer generation latched, later newer generation supersedes, abort/disconnect/timeout preserved.
- [ ] Footer/status presentation remains byte-identical; no watcher execution, polling, subscription, triggering, or cancellation behavior changes.
- [ ] Keep selector-aware formatting pure/domain-owned; avoid transport or result-schema breakage.
- [ ] Extend `observation-result.test.ts`, `tools.test.ts`, and composition/e2e coverage for the screenshot path and newer-generation completion.
- [ ] `pi-watcher` `make quick` and `make all` pass; record evidence in `.tmp/reports/`.

## Non-goals

Rejecting valid exclusive waits, changing timeout defaults, auto-triggering/cancelling work, changing footer wording, or fetching failure evidence during progress.
