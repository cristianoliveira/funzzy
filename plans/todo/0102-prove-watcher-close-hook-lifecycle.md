---
id: TASK-0102
title: Prove watcher close hook lifecycle
status: doing
depends_on: [TASK-0101]
priority: high
tags: [integration-tests, watcher, hooks, signals, process, config-reload, reliability]
---

# Prove watcher close hook lifecycle

## Problem
A shutdown hook can duplicate, hang, alter exit status, or be skipped across signal and fatal-error paths unless the installed binary is tested through real process shutdown.

## Context

Use compiled `fzz` in isolated temporary workspaces and existing bounded process/polling harness. Test observable files, output, exit status, and descendant cleanup; mocked signals do not prove this feature.

## Acceptance criteria

- [ ] Black-box SIGINT and SIGTERM tests prove close hook runs exactly once and original conventional exit status remains 130/143.
- [ ] Hook runs only after active finite jobs and managed services are cancelled/reaped; test records ordering without fixed sleeps.
- [ ] Hook success, command failure, spawn failure, timeout, and descendant process cases terminate within bound and leave no orphan.
- [ ] Hook failure is visible on stderr/diagnostics but does not replace signal or fatal-shutdown exit code.
- [ ] Second signal/concurrent fatal event cannot duplicate hook execution.
- [ ] Valid reload changes hook used at shutdown; invalid reload executes last valid committed hook exactly once while preserving fatal config exit.
- [ ] Config without close hook preserves current shutdown behavior and timing within a reasonable deterministic budget.
- [ ] Finite `fzz run` and representative non-watcher command do not execute configured close hook.
- [ ] File written by close hook does not trigger another generation after shutdown begins.
- [ ] `fzz config schema`, comprehensive init template, catalog parity, and parser rejection tests cover `on.close`.
- [ ] README and run-hooks contract show generic cleanup/notification example and explicitly distinguish close from success/failure.
- [ ] Focused integration tests, process-group tests, config reload matrix, lint, and final watcher verification pass.
- [ ] GitHub issue #234 can be closed with links to contract, implementation, and black-box evidence.

## Notes

Keep each fixture’s output paths unique and always clean spawned children on assertion failure.

