---
id: TASK-0133
title: Document service restart-by-reinclusion model and the init-only service footgun
status: todo
depends_on: []
priority: normal
tags: [rust, docs, services, config]
---

# Document service restart-by-reinclusion model and the init-only service footgun

## Problem

`service: true` restart-on-change is implemented as generation
replacement: the active generation is cancelled (services shut down) and
the superseding plan re-includes the service only when the event
matches its patterns — the per-job `change:` merged with root
`on.change` (`merge_patterns`, `src/config.rs:431`). That model works
and is by design (TASK-0035), but nothing tells the user it exists:

1. A service declared with only `run_on_init: true` (validator accepts
   `change` OR `run_on_init`) is guaranteed to die on the first
   scheduled generation and never come back. Silent footgun.
2. The supported idiom for a long-lived bridge/poller — declare
   `change: [<its own output file>]` so every publish re-includes it —
   is undiscoverable without reading the executor source.

## How to reproduce

Init-only service (footgun):

```yaml
jobs:
  - name: gh-actions mirror
    service: true
    run_on_init: true
    output: quiet
    run: sh .tmp/gh-test/mirror.sh   # poll loop writing runs.json
    env: { OUT: .tmp/gh-test/state/runs.json, INTERVAL: "1" }

  - name: ci status changed
    run: sh .tmp/gh-test/reactor.sh "{{filepath}}"
    change: [".tmp/gh-test/state/runs.json"]
```

12s run: the mirror's first publish fires the reactor generation, which
replaces (and kills) the init generation; the service is not in the new
path-filtered plan and stays dead (1 gh call, 1 event; evidence in
`.tmp/reports/26-08-26/gh-actions-watch-design-a.md`).

Working idiom (same config, service gains one line):

```yaml
    change: [".tmp/gh-test2/state/runs.json"]   # its own output
```

12s run (26-08-26, `.tmp/gh-test2/`): bridge started 4x (init + 3
re-inclusions), published 3 state transitions, reactor fired exactly 3
events with correct `{{filepath}}`. The cmp guard in the poller
prevents a restart loop (unchanged state is never rewritten).

## Expected

- Docs (USAGE/ADVANCED-GUIDE + `fzz config` service help) state the
  model: a service lives in the generations whose merged patterns it
  matches; cancel-and-respawn is the restart mechanism; common
  `on.change` triggers are how services rejoin generations.
- Docs show the bridge idiom: watch your own output file; keep the
  poller stateless (restart loses memory) and idempotent (guard
  against rewrite loops).
- Decision on the footgun, one of:
  - `fzz check` warns when a `service: true` job declares no `change:`
    (init-only services cannot survive generation churn), or
  - explicitly documented as unsupported shape.
- Adjacent note for the same docs page: a service present in a
  generation's plan keeps that generation alive (`Step::Running`), so
  the results summary renders only at shutdown/supersede.

## Current

- Behavior is consistent and by design; only the discoverability is
  missing. Working single-config bridge today: service with
  `change: [<own output>]` + reactor on the same path
  (`.tmp/gh-test2/watch.yaml`).
- Split-instance alternative (bridge-only config + reactor config)
  also works but needs two processes
  (`.tmp/gh-test/watch.bridge.yaml`, `watch.react.yaml`).
- Events matching only other jobs' exclusive patterns still kill a
  service without respawn (restart happens solely via re-inclusion);
  common triggers covering the edit surface make this rare — worth one
  sentence in the docs, not a behavior change.
