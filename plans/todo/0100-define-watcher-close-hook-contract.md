---
id: TASK-0100
title: Define watcher close hook contract
status: todo
depends_on: [TASK-0040]
priority: high
tags: [design, cli, config, hooks, shutdown, issue-234, determinism]
---

# Define watcher close hook contract

## Problem
GitHub issue #234 asks Funzzy to run user automation when a watcher closes, but existing success/failure hooks only describe generation outcomes and shutdown paths have no shared close-hook semantics.

## Context

Issue: https://github.com/cristianoliveira/funzzy/issues/234

Extend `docs/RUN-HOOKS-CONTRACT.md`. Use `on.close` as sibling of `on.success` and `on.failure`; nested YAML already supplies the “on” vocabulary, so `on_close` would repeat it.

## Acceptance criteria

- [ ] Contract distinguishes generation terminal hooks (`success`/`failure`) from watcher-session terminal hook (`close`).
- [ ] `on.close` is an optional finite shell command and is rejected outside supported preferred/legacy grouped config shapes like other `on` properties.
- [ ] Close hook runs exactly once after watcher stops accepting/scheduling work and active jobs/services are cancelled and reaped, but before Funzzy process exits.
- [ ] It applies to graceful SIGINT/SIGTERM, fatal runtime config shutdown, and any normal watcher return; SIGKILL and startup failure before watcher readiness cannot run it.
- [ ] Finite `fzz run`, `check`, `list`, `explain`, `exec`, `config`, `init`, `migrate`, and control clients never run watcher close hook.
- [ ] A reloaded watcher uses latest successfully committed `on.close`; malformed candidate never replaces last valid hook.
- [ ] Hook runs from workspace root with inherited environment and no synthetic `filepath`/generation identity; unsupported trigger templates fail or warn consistently with existing hook policy.
- [ ] Hook failure is visible but never replaces original watcher exit reason/code (130 SIGINT, 143 SIGTERM, nonzero fatal config).
- [ ] Hook receives bounded graceful execution; timeout/cancellation uses explicit deterministic shutdown policy and descendants are reaped.
- [ ] A second termination signal and internal concurrent shutdown requests cannot execute hook twice.
- [ ] Watcher is already quiescent, so files written by close hook do not schedule another generation.
- [ ] Output/diagnostic behavior and control-socket ordering are specified without inventing a fake generation ID.
- [ ] README/config schema/init catalog examples and issue #234 completion evidence are identified as compatibility surfaces.

## Notes

Close means watcher lifecycle close, not “run another command after every workflow.”

