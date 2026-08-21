---
id: TASK-0126
title: Report job durations in human control output
status: done
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

- [x] Write failing Rust and pi-watcher presentation tests first for terminal task durations, absent durations, failures, cancellation, and legacy snapshots.
- [x] Render job timing rows in human control results wherever a correlated terminal snapshot is available, including exact await/observation flows.
- [x] Preserve existing JSON/TOON `tasks[].durationMs` shape and integer value without adding duplicate duration fields.
- [x] Preserve job declaration order and state labels; do not reorder by duration or parallel completion time.
- [x] Keep generation `durationMs` visually and semantically separate from each job's duration.
- [x] Render absent duration as unknown/dash, never zero, for skipped jobs and legacy servers.
- [x] Keep compact agent output bounded: include timing rows in structured content while avoiding repeated prose or parallelism disclaimers.
- [x] Ensure failure evidence, freshness, generation identity, configured/effective concurrency, and next-action hints remain unchanged.
- [x] Do not make per-job duration availability a freshness guarantee or infer it by parsing NDJSON/log text.
- [x] Synchronize Rust fixtures and pi-watcher decoders/presenters when a shared golden payload changes.
- [x] Cover negotiated and legacy happy/unhappy paths with deterministic tests before implementation.

## Outcome

Terminal control, `watcher_observe`, and terminal `watcher_verify` reports now reuse ordered executor snapshots, show `-` for absent job duration, and keep legacy task availability unknown. The control wire keeps `tasks[].durationMs` integer-or-null with internal position omitted.

Evidence: Rust focused suites (`control_client` 30, `cli::control` 23, `cli::format` 10, `watcher_state` 7), Pi watcher focused tests (72), format/typecheck/lint, and `npm pack --dry-run` passed. The final watcher integration run exposed a TASK-0125-local-output-sensitive filepath-template assertion; it was not changed in this task.

## Notes

A new control method is unnecessary for first slice because correlated snapshots already contain measurements.
