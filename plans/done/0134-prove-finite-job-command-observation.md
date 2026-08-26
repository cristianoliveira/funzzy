---
id: TASK-0134
title: Prove finite jobs can observe a blocking command
status: done
depends_on: []
priority: high
tags: [investigation, integration-tests, jobs, control-socket, process, determinism]
---

# Prove finite jobs can observe a blocking command

## Problem

Before adding an external-watching abstraction, we need evidence that an ordinary finite Funzzy job already provides the required observation lifecycle. Without a black-box proof, we risk duplicating process supervision or changing managed-service semantics unnecessarily.

## User outcome

A user-provided script may talk to any system, remain alive while its result is pending, and exit when terminal. Funzzy should observe only that process: alive is running, exit `0` is passed, and non-zero is failed.

## Validation scenario

Use a deterministic local script controlled by a test gate or file. Do not call GitHub, a network API, or the wall clock as an assertion strategy.

Exercise both:

```sh
fzz run await-remote
fzz ctl run await-remote --format toon
```

The control scenario must retain the exact returned run identity for later await, output, and cancellation operations.

## Acceptance criteria

- [ ] A finite target remains observably running while its script is blocked.
- [ ] Releasing the script with exit `0` produces exactly one terminal passed result.
- [ ] Releasing the script with non-zero exit produces exactly one terminal failed result with bounded stdout/stderr evidence.
- [ ] Cancelling the exact blocked generation reaps the script and its descendants and reports cancellation without affecting a newer generation.
- [ ] Local `fzz run TARGET` and daemon `fzz ctl run TARGET` behavior are compared and differences recorded.
- [ ] The existing `--wait --timeout` behavior is recorded accurately: it bounds the client wait and does not terminate the child.
- [ ] Current configuration friction is demonstrated: a control-only job cannot be declared without a filesystem or init trigger, and root `on.change` is inherited.
- [ ] Findings identify only proven gaps and recommend whether the first user deliverable is documentation or a configuration change.
- [ ] No production behavior, provider integration, structured script protocol, service semantics, or task timeout is implemented in this task.

## Deliverable

A committed deterministic regression test when existing public behavior can be asserted without changing it, plus a concise report linked from follow-up tasks. Any discovered defect becomes a separate task rather than expanding this investigation.

## Findings

- `tests/finite_job_observation.rs` proves a plain finite target remains blocked until a user-owned script releases it, then maps exit 0 to one passed result and non-zero to one failed result with bounded stdout/stderr evidence.
- The same proof compares local `fzz run await-remote` with daemon `ctl run await-remote --wait --timeout`, retaining and asserting the exact scheduled generation identity.
- Existing `control_cancel.rs`, `control_output.rs`, `control_await.rs`, and `process_groups.rs` cover exact cancellation, descendant cleanup, bounded output, and client wait timeout semantics. Client `--timeout` remains an await deadline and does not terminate the child.
- Current config still requires a change pattern or init trigger for a target; explicit `run TARGET`, control `run`, `emit`, or a marker-file drop are viable existing composition seams. No production behavior or service semantics changed.
- Report: `.tmp/reports/26-08-26/integration-agnostic-command-watcher.md`.
