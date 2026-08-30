---
id: TASK-0156
title: Make fzz check validate generation hooks
status: todo
depends_on: []
priority: high
tags: [cli, check, config, hooks, validation, tdd]
---

# Make fzz check validate generation hooks

## Problem
fzz check reports a configuration as valid even when watch startup rejects the same file because hooks.success or hooks.failure has an invalid shape. This violates check command promise to use watch-time parsers and delays actionable feedback until runtime.

## Desired outcome
`fzz check` rejects every generation-hook shape that normal watcher startup rejects, using the same production parser and actionable diagnostic.

## Acceptance criteria
- [ ] A black-box test first proves `fzz check` fails for an invalid scalar generation hook such as `hooks.failure: 1`.
- [ ] Invalid settled-hook objects (missing/invalid `run`, missing/zero/invalid `settle`, or unknown properties) fail through the same `generation_hooks_from_file` parser used at watcher startup.
- [ ] Failure exits nonzero, reports `Invalid hooks config` plus the parser reason, and never prints `config valid`.
- [ ] Valid scalar `hooks.success`/`hooks.failure` and valid `{ run, settle }` failure hooks continue to pass `fzz check`.
- [ ] Legacy accepted `on.success`/`on.failure` placement remains validated through the same compatibility path.
- [ ] `fzz check` remains side-effect-free: it starts no watcher, task, or control socket.
- [ ] Focused `check` CLI tests and generation-hook parser tests pass on happy and unhappy paths.

## Non-goals
- Changing which generation-hook shapes are valid.
- Completing settled-hook execution lifecycle in TASK-0153.
- Changing runtime hook commands, socket behavior, migrations, or config schema output.
- Refactoring every config loader into a new aggregate abstraction.

## Notes
- Root cause: `src/app.rs::check_config` validates session hooks but never calls `config::generation_hooks_from_file`; watcher startup calls it through `load_hooks`.
- Prefer one direct validation call over duplicating hook-shape rules in the command layer.
- Nearby black-box check tests live in `tests/cli_arguments.rs`; parser characterization tests live in `src/config.rs`.

