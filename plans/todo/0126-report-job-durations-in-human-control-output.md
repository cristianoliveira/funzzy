---
id: TASK-0126
title: Report job durations in human control output
status: todo
depends_on: [TASK-0124]
priority: high
tags: [rust, typescript, control-socket, duration, output, tdd]
---

# Report job durations in human control output

## Problem
Correlated control snapshots already carry `tasks[].durationMs`, but human control and Pi watcher summaries hide those values, forcing users to inspect raw structured payloads.

## Context

This is primarily presentation change. Preserve existing additive wire field and capability negotiation; legacy servers without task snapshots remain clearly weaker rather than receiving invented timings.

## Acceptance criteria

- [ ] Write failing Rust and pi-watcher presentation tests first for terminal task durations, absent durations, failures, cancellation, and legacy snapshots.
- [ ] Render job timing rows in human control results wherever a correlated terminal snapshot is available, including exact await/observation flows.
- [ ] Preserve existing JSON/TOON `tasks[].durationMs` shape and integer value without adding duplicate duration fields.
- [ ] Preserve job declaration order and state labels; do not reorder by duration or parallel completion time.
- [ ] Keep generation `durationMs` visually and semantically separate from each job's duration.
- [ ] Render absent duration as unknown/dash, never zero, for skipped jobs and legacy servers.
- [ ] Keep compact agent output bounded: include timing rows in structured content while avoiding repeated prose or parallelism disclaimers.
- [ ] Ensure failure evidence, freshness, generation identity, configured/effective concurrency, and next-action hints remain unchanged.
- [ ] Do not make per-job duration availability a freshness guarantee or infer it by parsing NDJSON/log text.
- [ ] Synchronize Rust fixtures and pi-watcher decoders/presenters when a shared golden payload changes.
- [ ] Cover negotiated and legacy happy/unhappy paths with deterministic tests before implementation.

## Notes

A new control method is unnecessary for first slice because correlated snapshots already contain measurements.
