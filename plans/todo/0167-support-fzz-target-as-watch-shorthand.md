---
id: TASK-0167
title: Support fzz -- TARGET as watch shorthand
status: todo
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

- [ ] Add parser tests first for `fzz -- TARGET`, delimiter-only, option composition, a subcommand-shaped target, a hyphen-prefixed target, and too many trailing values.
- [ ] Map the root trailing target to the same `Action::Watch` fields used by `fzz watch TARGET`; do not add a second selection/execution path.
- [ ] Preserve repeatable root `--exclude` order and `--no-services` composition before the delimiter.
- [ ] Prove missing and ambiguous targets retain existing watch-target diagnostics and exit behavior without falling back to all jobs.
- [ ] Prove extra trailing values and incompatible root actions fail with exit 2 before configuration roots, sockets, jobs, or services start.
- [ ] Add a spawned-watcher test showing `fzz -- @quick` runs only the selected configured jobs and has the same effective plan/control status as `fzz watch @quick`.
- [ ] Add a spawned composition test for `fzz --no-services -- @quick`, proving retained finite work completes while services never start or appear in status.
- [ ] Preserve byte/behavior compatibility for zero-argument watch, explicit subcommands, `exec --`, help/version precedence, and no-delimiter unknown subcommands.
- [ ] Update root help, shell completions, README, usage, target, migration, and release-boundary documentation with the shorthand and delimiter semantics.
- [ ] Run focused parser tests, feature-gated integration tests, documentation/config drift checks, lint/format, and the configured final gate.

## Non-goals

- Accepting `fzz TARGET` without `--`.
- Changing target matching or exclusion vocabulary.
- Making `fzz -- TARGET` run a finite job.
- Forwarding multiple arguments as an ad-hoc command; use `fzz exec -- PROGRAM ARG...`.
- Changing YAML, control-socket, or Pi-watcher protocols.

## Implementation constraint

Model the shorthand as one optional root positional that is accepted only after the end-of-options delimiter, then normalize it into the existing `Action::Watch`. Do not infer behavior by manually scanning raw `argv` if Clap can express and validate the grammar deterministically.

