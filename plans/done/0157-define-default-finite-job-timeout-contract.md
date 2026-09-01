---
id: TASK-0157
title: Define default finite-job timeout contract
status: done
depends_on: []
priority: high
tags: [design, timeout, jobs, config, determinism]
---

# Define default finite-job timeout contract

## Problem
Users can bound one finite job today, but must repeat the same timeout on every job; omitted values stay unbounded and can let a slow pipeline hang indefinitely. Funzzy needs one explicit default with predictable per-job override, service, compatibility, and observability semantics.

## Desired outcome

A user can set one default execution timeout for finite jobs and override it only where a job needs a different budget. A timed-out job keeps the existing typed `timedout` outcome and fails the run/generation so the user can investigate retained evidence.

## Accepted API

```yaml
execution:
  timeout: 10m
jobs:
  - name: lint        # inherits 10m
    run: cargo clippy
  - name: integration
    timeout: 30m      # job override wins
    run: cargo test --test integration
```

## Acceptance criteria

- [x] Use optional `execution.timeout` with the existing positive-duration grammar and actionable validation; absence preserves current behavior.
- [x] Define precedence explicitly: `jobs[].timeout` overrides `execution.timeout`; a finite job with neither remains unbounded for backward compatibility.
- [x] Do not add an opt-out sentinel in this change. An omitted job timeout inherits the execution default; `timeout: null`, zero, and sentinel strings remain invalid.
- [x] Keep the existing deadline start, monotonic clock, process-group termination, duration accounting, `timedout` task state, failed generation, exit code, hooks, retained output, and recovery behavior unchanged.
- [x] An execution default applies only to finite jobs. `service: true` remains unbounded; the existing direct `jobs[].timeout` plus service validation remains intact.
- [x] Keep job execution timeout distinct from control-client `--timeout` and from any whole-generation wall-time limit in terminology and output.
- [x] Freeze each resolved effective timeout into its scheduled generation; a timeout-only hot reload affects later generations only.
- [x] Preserve legacy root-list/grouped `tasks:` compatibility and existing V2 configurations that omit `execution.timeout`.
- [x] Update the canonical option catalog, schema, generated init/example content, config rendering, revision identity, README/usage/advanced guidance, and tests.
- [x] No control-protocol or pi-watcher wire change is required because effective jobs reuse the existing `timedout` state.

## Non-goals

- A whole-pipeline or whole-generation deadline.
- Changes to control-client await deadlines.
- Deadlines for managed services.
- Changes to timeout termination or outcome semantics already established by TASK-0138.
- A per-job `unbounded`/`none` opt-out from a configured default.

## Outcome

Accepted on 2026-09-01. Implementation proceeds through TASK-0158, followed by black-box proof and documentation in TASK-0159.

## Related work

- TASK-0138: finite-job timeout contract.
- TASK-0139: per-job timeout implementation.
- TASK-0140: per-job timeout proof and documentation.
