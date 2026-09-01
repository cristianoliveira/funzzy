---
id: TASK-0157
title: Define default finite-job timeout contract
status: todo
depends_on: []
priority: high
tags: [design, timeout, jobs, config, determinism]
---

# Define default finite-job timeout contract

## Problem
Users can bound one finite job today, but must repeat the same timeout on every job; omitted values stay unbounded and can let a slow pipeline hang indefinitely. Funzzy needs one explicit default with predictable per-job override, service, compatibility, and observability semantics.

## Desired outcome

A user can set one default execution timeout for finite jobs and override it only where a job needs a different budget. A timed-out job keeps the existing typed `timedout` outcome and fails the run/generation so the user can investigate retained evidence.

## Preferred API to evaluate

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

- [ ] Define the execution-level default syntax, positive-duration grammar, validation errors, absence behavior, and canonical schema/help representation.
- [ ] Define precedence explicitly: `jobs[].timeout` overrides the execution default; a finite job with neither remains unbounded for backward compatibility.
- [ ] Decide whether a finite job can explicitly opt out of a configured default; if supported, define one unambiguous syntax and validation behavior rather than overloading zero.
- [ ] Keep the existing deadline start, monotonic clock, process-group termination, duration accounting, `timedout` task state, failed generation, exit code, hooks, retained output, and recovery behavior unchanged.
- [ ] Define managed-service behavior explicitly. An execution default must not accidentally terminate `service: true`; the existing direct `jobs[].timeout` plus service validation remains intact.
- [ ] Keep job execution timeout distinct from control-client `--timeout` and from any whole-generation wall-time limit in terminology and output.
- [ ] Define how the effective timeout is frozen into a scheduled generation and how timeout-only hot reload changes affect later generations.
- [ ] Preserve legacy root-list/grouped `tasks:` compatibility and existing V2 configurations that omit the new default.
- [ ] Identify required updates to the canonical option catalog, schema, generated init/example content, config rendering, revision identity, README/usage/advanced guidance, and tests.
- [ ] Record whether this additive configuration field requires any control-protocol or pi-watcher change; do not expand wire scope without evidence.

## Non-goals

- A whole-pipeline or whole-generation deadline.
- Changes to control-client await deadlines.
- Deadlines for managed services.
- Changes to timeout termination or outcome semantics already established by TASK-0138.

## Related work

- TASK-0138: finite-job timeout contract.
- TASK-0139: per-job timeout implementation.
- TASK-0140: per-job timeout proof and documentation.
