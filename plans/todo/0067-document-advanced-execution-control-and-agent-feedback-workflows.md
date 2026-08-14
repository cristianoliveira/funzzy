---
id: TASK-0067
title: Document advanced execution control and agent feedback workflows
status: todo
depends_on: [TASK-0065, TASK-0028, TASK-0055, TASK-0059, TASK-0070, TASK-0074]
priority: high
tags: [docs, parallel, control-socket, agents, diagnostics, duration]
---

# Document advanced execution control and agent feedback workflows

## Problem
Parallel groups, wait/restart policy, process ownership, control methods, freshness, output evidence, cancellation, duration estimates, and pi-watcher behavior exist across implementation contracts but lack task-oriented user documentation.

## Context

Convert normative contracts into user goals and operational recipes while linking contracts for exact compatibility semantics.

## Acceptance criteria

- [ ] Parallel guide explains named contiguous groups, barriers, filtering, command sequentiality, `on.concurrency`, `--sequential` comparison, failure/fail-fast, output ordering, and workload tradeoffs with measured example.
- [ ] Control guide documents canonical `control` plus visible `ctl` alias, socket precedence, capabilities, status/list/run/emit/await/output/cancel, exact identity/freshness, timeout, exit codes, and bounded evidence.
- [ ] Agent guide gives compact edit-feedback loop: capabilities → observe → edit/emit/run → atomic exact await → output diagnosis → exact cancel, including stale/restart/fallback handling.
- [ ] Duration guide explains local XDG history, eligibility, confidence, timeout precedence, reset/privacy, invalidation, and `slower-than-history` without promising ETA.
- [ ] pi-watcher section clearly separates Funzzy execution truth from Pi projection and capability-gated legacy fallback.
- [ ] Troubleshooting covers no match, ignored path, ambiguous target, unavailable/stale socket, superseded generation, truncated output, process cleanup, config reload, and corrupt history.
- [ ] Protocol JSON examples are golden fixtures or generated from tests and remain bounded/redacted.
- [ ] Recipes prove local run, control run, synthetic deletion emit, failure evidence, cancellation, parallel verification, and multi-session ownership where public.
- [ ] No page reconstructs freshness via polling or teaches deprecated compatibility paths.

## Notes

