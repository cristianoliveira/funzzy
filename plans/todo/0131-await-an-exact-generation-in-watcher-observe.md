---
id: TASK-0131
title: Await an exact generation in watcher_observe
status: todo
depends_on: []
priority: high
tags: [typescript, pi-watcher, observer, polling, axi, tdd]
---

# Await an exact generation in watcher_observe

## Problem
Agents that discover an already-running generation can only pass afterGeneration today, which excludes that generation. The tool then keeps polling after it reports that same generation passed, producing misleading updates such as PASS gen=37 waited=475s.

## Context

`watcher_observe(wait: true, afterGeneration: 37)` means “ignore generation 37 and wait for first generation >37.” On legacy polling fallback, progress currently keeps rendering terminal generation 37 as `PASS ... waited=...`, although application correctly excludes it. Agent has no exact-generation selector, so already-running generation is easy to anchor incorrectly.

Small API addition:

```text
watcher_observe wait=true generation=37
```

Keep `afterGeneration` for pre-edit snapshots. Exact `generation` and `afterGeneration` are mutually exclusive.

## Acceptance criteria
- [ ] Add failing application tests reproducing terminal generation equal to `afterGeneration` being excluded while progress misleadingly appears passed or failed.
- [ ] Add optional non-negative `generation` to `WatcherObserveRequest` and public TypeBox tool schema.
- [ ] With `wait=true, generation=G`, return when G is terminal; keep waiting while observed generation is G and running.
- [ ] If observer first/next sees generation greater than G before terminal G, return explicit `superseded` with newer generation.
- [ ] Reject `generation` together with `afterGeneration`, and reject `generation` when `wait` is false, with self-correcting parameter guidance.
- [ ] Keep existing anchor mode (`wait=true` without selector) and fresh mode (`afterGeneration`) behavior unchanged.
- [ ] Make progress text state selector semantics: exact waits show `generation=G`; fresh waits that observe excluded terminal anchor say `waitingForGeneration>G` rather than presenting misleading PASS as completion.
- [ ] Update prompt guidelines: use `afterGeneration` captured before edit; use exact `generation` (or `watcher_status(wait=true)`) for a run already observed as active.
- [ ] Cover passed and failed terminal outcomes across subscription and legacy polled sources; exact G must complete immediately and preserve bounded failure evidence rather than waiting caller timeout.
- [ ] Run pi-watcher focused tests and final watcher gate.

## Notes

Do not lower global timeout or reinterpret `afterGeneration`; both would hide caller error or introduce a race before a post-edit generation is scheduled. Subscription connect cancellation remains separate transport work.

