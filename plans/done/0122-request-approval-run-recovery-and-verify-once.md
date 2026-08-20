---
id: TASK-0122
title: Request approval, run job recoveries, and verify once
status: done
depends_on: [TASK-0121]
priority: high
tags: [rust, executor, approval, recovery, cancellation, output, tdd]
---

# Request approval, run job recoveries, and verify once

## Problem
Even when recovery commands are configured, Funzzy has no safe, correlated, and deterministic lifecycle for asking the user, recovering a failed job, verifying it once, or reporting the final outcome.

## Context

Add approval as an injected execution boundary rather than reading global stdin from domain code. Foreground CLI composition owns the TTY adapter; executor consumes an approval decision tied to exact generation and job identity. This keeps tests deterministic and prevents stale input from authorizing unrelated work.

Recovery commands mutate the workspace. They must run exclusively: an initial parallel stage may complete concurrently, but its eligible failed jobs enter approval/recovery/verification in declaration order after sibling original attempts stop.

## Acceptance criteria

- [ ] Write failing executor tests first for original pass, explicit skip policy, decline, no TTY, approval, recovery pass + verification pass, recovery failure, verification failure, multiple recovery commands, and ordinary jobs without recovery.
- [ ] Thread the frozen `recovery_policy` through finite run and watch composition; `skip` bypasses the approval port and immediately preserves the original failure.
- [ ] Add `--recovery-policy prompt|skip` as an explicit run/watch override with CLI-over-config precedence and actionable invalid-value errors.
- [ ] Introduce a small injected approval port carrying exact generation, config revision, job position/name, and rendered command set; production TTY input/output remains at the CLI boundary.
- [ ] Render a default-deny prompt with exact job and commands; accept the contract's affirmative answer and reject/decline all other input without executing commands.
- [ ] Keep CI and headless execution finite: explicit `skip` and missing TTY return immediately with distinct actionable diagnostics instead of blocking socket-run watchers or detached processes; do not infer policy from `CI=true`.
- [ ] Run approved recovery commands once, sequentially, fail-fast, with the job's resolved cwd/environment/templates/output policy and existing shell/process-group ownership.
- [ ] Rerun the complete original job command list exactly once only when every recovery command succeeds.
- [ ] Preserve serial barriers; for a parallel stage, wait for all original attempts, then process eligible failed jobs exclusively and in declaration order.
- [ ] Delay terminal task failure, global fail-fast, stage advancement, and generation failure hook selection until approval/recovery/verification resolves the job's final outcome.
- [ ] Emit/retain distinguishable original, approval, recovery, and verification evidence without duplicating a terminal `TaskTerminal` event or hiding the initial failure.
- [ ] Ensure output policies remain truthful: `show-on-failure` reveals evidence only for final failure; successful recovery remains inspectable through verbose/structured retained output.
- [ ] Cancel and reap an active recovery or verification exactly like ordinary commands; cancel/supersede invalidates pending approval and never lets a late response approve a later generation/job.
- [ ] Keep generation identity and config revision frozen across original attempt, prompt, recovery, and verification; hot reload applies only to later generations.
- [ ] Ensure success/failure hooks run once from final generation outcome and hook failures never trigger job recoveries.
- [ ] Add deterministic tests for parallel declaration-order prompts, concurrent sibling completion, restart while awaiting approval, cancellation during recovery, and stale affirmative input.
- [ ] Verify both blocking and restart-capable worker strategies use the same executor policy rather than duplicating approval logic.

## Notes

Remote approval through `fzz ctl` and unattended `--accept-recoveries` are out of scope. The approval port should permit those adapters later without changing executor policy.
