---
id: TASK-0155
title: Use .watch.sock in generated init configuration
status: done
depends_on: []
priority: normal
tags: [cli, init, config, socket, tdd]
---

# Use .watch.sock in generated init configuration

## Problem
Fresh `fzz init` configurations currently teach `.tmp/funzzy/control.sock`, which creates an unnecessary nested runtime path and makes the conventional project-local control socket harder to discover. Generated configurations should use the concise project-root `.watch.sock` path without changing existing user configurations or runtime fallback behavior.

## Desired outcome
A newly initialized configuration that enables the control socket points to `.watch.sock` by default.

## Acceptance criteria
- [x] Plain `fzz init` writes an active `on.socket: .watch.sock` entry.
- [x] Generated socket-enabled profiles and canonical option examples use `.watch.sock`; they do not emit `.tmp/funzzy/control.sock`.
- [x] `fzz init` remains create-only and does not create the socket itself.
- [x] All generated profiles still parse and validate, and `fzz init --template PROFILE` remains byte-identical to `fzz config example PROFILE`.
- [x] Existing configuration files and explicitly configured socket paths are unchanged.
- [x] Focused `command_init*` and template/profile tests cover the new default and absence of the old path.

## Non-goals
- Changing socket runtime lifecycle, cleanup, RPC protocol, or CLI flags.
- Migrating existing `.watch.yaml` files.
- Adding a fallback socket when `on.socket` is absent.

## Notes
- Current generated references live in `src/cli/init.rs`, `src/cli/templates.rs`, and `src/option_catalog.rs`.
- Keep one canonical generated socket-path constant if that avoids future profile drift.
- Implemented by `cd87838`; black-box closure proof in `343f427`.
- Independent focused verification: command-init proof 4 passed, init unit tests 11 passed, template unit tests 5 passed.
- Watcher generation 50 exposed an unrelated pre-existing semantic-hash fixture failure, reproduced at pre-task commit `24528dd`.

