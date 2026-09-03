---
id: TASK-0167
title: Support fzz -- TARGET as watch shorthand
status: done
depends_on: [TASK-0165]
priority: high
tags: [cli, watch, targets, compatibility, docs, tdd]
---

# Support fzz -- TARGET as watch shorthand

## Problem

The zero-argument command starts the configured watcher, but selecting a target requires the longer `fzz watch TARGET` form. Users want `fzz -- TARGET` as a concise, explicit shorthand without making arbitrary root positionals compete with subcommand names.

## Desired outcome

`fzz -- TARGET` behaves exactly like `fzz watch TARGET`, including target resolution, exclusions, lifecycle, output, reload policy, errors, and exit codes. The delimiter keeps the command tree unambiguous.

## Syntax contract

- `fzz -- TARGET` is a watch-target alias, not finite `run` and not ad-hoc `exec`.
- `fzz --` remains equivalent to zero-argument `fzz`; the delimiter alone is harmless.
- Root watch options compose before the delimiter: `fzz --no-services -- @quick` and `fzz --exclude lint -- @quick`.
- Everything after `--` is target data. For example, `fzz -- --service` selects a task named `--service`; it does not enable an option.
- Exactly zero or one target is accepted after the delimiter. Extra values fail with Clap exit 2 and never start a watcher.
- `fzz -- watch` selects a configured target named `watch`; it does not invoke the `watch` subcommand.
- Existing `fzz watch TARGET`, `fzz run TARGET`, and `fzz exec -- PROGRAM ARG...` behavior remains unchanged.

## Acceptance criteria

- [x] Add parser tests first for `fzz -- TARGET`, delimiter-only, option composition, a subcommand-shaped target, a hyphen-prefixed target, and too many trailing values.
- [x] Map the root trailing target to the same `Action::Watch` fields used by `fzz watch TARGET`; do not add a second selection/execution path.
- [x] Preserve repeatable root `--exclude` order and `--no-services` composition before the delimiter.
- [x] Prove missing targets retain existing watch-target diagnostics and multi-match targets retain existing watch selection without falling back to all jobs.
- [x] Prove extra trailing values and incompatible root actions fail with exit 2 before configuration roots, sockets, jobs, or services start.
- [x] Add a spawned-watcher test showing `fzz -- @quick` runs only the selected configured jobs and has the same effective plan/control status as `fzz watch @quick`.
- [x] Add a spawned composition test for `fzz --no-services -- @quick`, proving retained finite work completes while services never start or appear in status.
- [x] Preserve byte/behavior compatibility for zero-argument watch, explicit subcommands, `exec --`, help/version precedence, and no-delimiter unknown subcommands.
- [x] Update root help, shell completions, README, usage, target, migration, and release-boundary documentation with the shorthand and delimiter semantics.
- [x] Run focused parser tests, feature-gated integration tests, documentation/config drift checks, lint/format, and the configured final gate.

## Non-goals

- Accepting `fzz TARGET` without `--`.
- Changing target matching or exclusion vocabulary.
- Making `fzz -- TARGET` run a finite job.
- Forwarding multiple arguments as an ad-hoc command; use `fzz exec -- PROGRAM ARG...`.
- Changing YAML, control-socket, or Pi-watcher protocols.

## Implementation constraint

Model the shorthand as one optional root positional that is accepted only after the end-of-options delimiter, then normalize it into the existing `Action::Watch`. Do not infer behavior by manually scanning raw `argv` if Clap can express and validate the grammar deterministically.

## Evidence

`src/arguments.rs` adds one Clap `root_target` positional with `.last(true)` and maps it directly into the existing `Action::Watch`. `tests/target_shorthand.rs` covers parser-compatible spawned behavior, exact plan/status parity, no-services composition, missing-target diagnostics, multi-match compatibility, and pre-load extra-value rejection. Generated shell completions include the root positional from the shared command tree. Documentation is updated in README, usage, target migration, migration, and release-boundary surfaces.

Focused evidence: `cargo test --lib` passed with 875 tests; `cargo test --lib arguments::tests -- --nocapture` passed with 83 tests; `cargo test --test cli_arguments -- --nocapture` passed with 53 tests; `cargo test --test target_shorthand --features test-integration --no-default-features -- --nocapture` passed with 5 tests; `cargo test --test command_init_proof --features test-integration --no-default-features -- --nocapture` passed with 5 tests; `cargo test --test config_command_workflow --features test-integration --no-default-features -- --nocapture` passed with 6 tests; `make lint` passed; and configured serial integration `cargo test --features test-integration --test '*' -- --nocapture --test-threads=1` passed. `git diff --check origin/task/0165-proof...HEAD` passes.

