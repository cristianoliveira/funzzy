---
id: TASK-0164
title: Implement watch exclusions and no-services mode
status: done
depends_on: [TASK-0163]
priority: high
tags: [rust, cli, targets, services, worktrees, tdd]
---

# Implement watch exclusions and no-services mode

## Problem

Funzzy needs a deterministic invocation-level way to run the configured workflow
except selected jobs, including a simple shortcut that excludes every
`service: true` job.

## Desired outcome

The TASK-0163 contract is available through the real CLI and shared planning
path, with excluded jobs removed before execution can acquire process ownership.

## Acceptance criteria

- [x] Add parser/help support for approved repeatable `--exclude TARGET` and `--no-services` on `fzz watch` and the zero-argument watch alias; `fzz run` rejects watch-only flags.
- [x] Resolve exclusions through existing target-selection rules: exact job name, `@tag`, or unambiguous name substring.
- [x] Filter excluded jobs while preserving remaining declaration order, group boundaries, signatures, and path matching through `RunPlan::filter`.
- [x] Ensure `--no-services` filters both legacy and readiness-enabled `service: true` jobs before roots, spawn, readiness, handoff, or pool reconciliation.
- [x] Compose positive target selection before repeated exclusions and `--no-services`; repeats/overlaps are idempotent. Root/subcommand mixed placement preserves encounter order.
- [x] Return actionable exit-2 errors for ambiguous, unmatched, or empty effective selections without falling back to all jobs.
- [x] Preserve behavior and startup diagnostics when neither exclusion option is present.
- [x] Add focused happy/unhappy parser and planning tests, including reload candidate policy tests.

## Implementation and reload policy

`Watches::select_target_with_exclusions` is the single planning boundary. It
resolves selectors against the original unfiltered rules, applies positive
selection first, then filters the effective topology while retaining parallel
group occurrences and barriers. The resulting `Watches` is passed to watch
execution only after selection errors have returned.

Invocation policy is carried in `ReloadSettings` and reapplied to every
validated reload candidate before `ReloadCoordinator::begin`, root diff, or
service diff. Selector strings are re-resolved against each candidate; missing,
newly ambiguous, or empty effective candidates fail the reload rather than
widening to all jobs. `--no-services` dynamically excludes newly added service
jobs as well. The policy is not part of `RuntimeConfig` or its revision hash.

`--exclude` supports both root-level options for the configured-watch alias and
subcommand options after `watch`; values concatenate in encounter order. No
watch-only options are accepted by local `fzz run`, control protocol, `list`, or
`explain`.

## Verification evidence

- `cargo test --lib`: 869 passed.
- `cargo test --test cli_arguments --features test-integration`: 57 passed,
  including alias parsing, mixed placement, run rejection, unknown exclusion,
  empty `--no-services`, and no-startup assertions.
- `cargo clippy --all-targets -- -D warnings`: passed.
- `cargo fmt -- --check` and `git diff --check`: passed.
- Additional planning tests cover exact names, `@tag`, unambiguous substrings,
  ambiguity, missing selectors, repeats, overlaps, legacy/readiness services,
  group preservation, candidate service filtering, candidate disappearance,
  candidate ambiguity, and empty candidate policy.

## Non-goals

- Coordinating service ownership across watcher processes.
- Persisting exclusions into `.watch.yaml` or hot-reload revisions.
- Adding service-specific telemetry.

## Constraints

Use the shared planning/filtering boundary. Do not scatter CLI exclusion checks
through executor or worker lifecycle code.

## Handoff

TASK-0165 remains blocked until this implementation PR merges. It must add full
spawned-watcher evidence for excluded services, control status/generation
behavior, and README/help documentation validation.
