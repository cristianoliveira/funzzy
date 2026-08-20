---
id: TASK-0120
title: Define user-approved job recovery contract
status: done
depends_on: []
priority: high
tags: [design, config, jobs, recovery, approval, determinism]
---

# Define user-approved job recovery contract

## Problem
A failing Funzzy job can only report failure. Users cannot declare a safe recovery command for a recoverable job, inspect it after failure, approve it, and have Funzzy verify the original job again.

## Context

Proposed preferred V2 shape:

```yaml
execution:
  recovery_policy: prompt # prompt | skip

jobs:
  - name: format-check
    run: cargo fmt --all -- --check
    recovery: cargo fmt --all
```

`recovery` accepts the same shell-command scalar or ordered list shape as `run`. Declaring it only makes a job eligible for recovery; it is not permission to mutate files. `execution.recovery_policy` selects whether Funzzy asks or skips recoveries. After the first failed attempt under `prompt`, Funzzy displays the exact command set and asks the attached user to approve it with a default-deny `y/N` prompt.

CI can make its non-mutating intent explicit with `fzz --recovery-policy skip ...`; CLI policy overrides configuration. A missing TTY also forces safe skip, without guessing from ambient `CI` environment variables.

MVP lifecycle:

```text
run job -> pass
        -> fail -> ask user
                     -> decline/no TTY/cancel -> final failure
                     -> approve -> run recovery commands once
                                      -> recovery fails -> final failure
                                      -> recovery passes -> rerun original job once
                                                          -> final pass/failure
```

This is deliberately bounded: one approval, one recovery pass, and one verification rerun. There is no recursive recovery or configurable retry loop.

## Acceptance criteria

- [x] Publish a normative contract defining `recovery` as an optional job-local scalar or ordered command list in preferred `jobs:` configuration.
- [x] Define `execution.recovery_policy: prompt | skip`, default `prompt`, plus exact CLI override `--recovery-policy`; CLI overrides config and no ambient `CI` variable silently changes behavior.
- [x] Define `skip` as immediate final failure: do not prompt, do not execute recovery commands, and emit a concise reason suitable for CI.
- [x] State clearly that configuration declares an available recovery, while explicit interactive acceptance authorizes each execution.
- [x] Define the prompt: identify exact generation and job, render all commands in execution order, and default to `No`; accept only an unambiguous affirmative response.
- [x] Define non-interactive behavior: explicit `skip`, missing TTY, EOF, invalid answer, or declined approval never runs the recovery and preserves the original job failure with an actionable diagnostic.
- [x] Define one bounded lifecycle: failed original attempt → approval → recovery commands once → original job once; no second prompt or recovery after recovery/verification failure.
- [x] Define recovery command behavior: sequential and fail-fast, same resolved `cwd`, environment, templates, cancellation, output capture, and process ownership as the job.
- [x] Reject `recovery` on `service: true` jobs because a service has no finite verification boundary.
- [x] Define parallel safety: finish the parallel stage's original attempts first, then prompt/recovery failed jobs exclusively and in declaration order so mutating recoveries never overlap sibling work or each other.
- [x] Define final outcome semantics: fail-fast and generation `hooks.success`/`hooks.failure` react only to the post-decision final result, while the initial failure and approval/recovery phases remain observable.
- [x] Define cancellation/restart behavior: a superseded or cancelled generation invalidates its pending approval; stale input cannot authorize another generation or job.
- [x] Define control-socket behavior for MVP: a generation remains running while awaiting its watcher's attached TTY; a headless watcher safely declines rather than waiting forever; remote approval is explicitly out of scope.
- [x] Record compatibility and versioning impact for JSON Schema, config reload/revision hashes, execution signatures, retained output, structured events, and pi-watcher.
- [x] List non-goals: matching particular exit codes/output text, automatic acceptance flags/policies, magic CI detection, remote approval, multiple retries, and generation-level recoveries.

## Outcome

Normative decision recorded in [`docs/JOB-RECOVERY-CONTRACT.md`](../../docs/JOB-RECOVERY-CONTRACT.md): preferred V2 jobs may declare bounded, job-local recoveries; default-deny interactive approval is required under `prompt`; `skip` and headless execution preserve the original failure; and one generation has at most one recovery pass and one verification.

## Notes

This capability differs from `hooks.failure`: a failure hook observes a final generation failure and cannot change its outcome; a job `recovery` runs before the job becomes terminal and may make verification pass.
