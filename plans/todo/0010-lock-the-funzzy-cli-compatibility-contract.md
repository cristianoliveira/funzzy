---
id: TASK-0010
title: Lock the Funzzy CLI compatibility contract
status: todo
depends_on: []
priority: high
tags: [rust, cli, tests]
---

# Lock the Funzzy CLI compatibility contract

Issue: https://github.com/cristianoliveira/funzzy/issues/226

## Problem

`src/app.rs` mixes Docopt parsing, a second-parse workaround for value-less `--target`, and application dispatch. Replacing parser without black-box contract tests risks silently changing accepted commands, short flags, output, or exit behavior.

## Deliverable

Executable characterization tests that define parser-visible compatibility before production parsing changes.

## Scope

- New focused `tests/cli_arguments.rs`
- Existing CLI fixtures where reuse keeps tests deterministic
- No parser or watcher production changes

## Acceptance criteria

- [ ] Both binaries, `funzzy` and `fzz`, have smoke coverage.
- [ ] `--help`, `-v`, and `--version` behavior is covered; `-V` remains verbose rather than Clap's default version short flag.
- [ ] Combined short flags such as `-nb` remain accepted.
- [ ] `--target` has covered absent, value-less, matching-value, and no-match paths.
- [ ] Configured watch, `watch '<command>'`, and direct `'<command>'` forms are covered.
- [ ] Supported option placement around command forms is explicit.
- [ ] Unknown options and missing required values have deterministic failure assertions.
- [ ] Assertions protect semantic help/error content and exit status without snapshotting colors or incidental whitespace.
- [ ] Any intentionally accepted Clap-native help/error formatting change is named in test comments rather than hidden as drift.

## Verification

- `cargo test --test cli_arguments`
- Existing `watching_filtered_tasks_with_target_flag` and `command_init*` tests remain green.
