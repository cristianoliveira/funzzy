---
id: TASK-0033
title: Validate configuration without starting a watcher
status: todo
depends_on: [TASK-0015, TASK-0025, TASK-0031, TASK-0032, TASK-0076]
priority: high
tags: [rust, cli, config, validation, diagnostics, tdd]
---

# Validate configuration without starting a watcher

## Problem
Users cannot check schema, globs, paths, parallel topology, and runtime settings without launching a long-running watcher and discovering errors incrementally.

## Context

Add `fzz check [--config PATH]` as side-effect-free command. It may inspect filesystem metadata but never starts watcher, commands, or socket.

## Acceptance criteria

- [ ] Black-box tests first cover valid config, YAML/schema error, invalid glob/duration/concurrency/group/context, missing path, and multiple errors.
- [ ] Command loads same parser/validator as watch; no duplicate validation implementation.
- [ ] Reports all independent actionable errors in deterministic config order where safe.
- [ ] Human output is concise and exit code is 0 valid, documented nonzero invalid/operational.
- [ ] Optional machine-readable output follows project CLI output contract if adopted.
- [ ] No task executes and no watcher/socket/log side effect occurs.
- [ ] Success reports config path and configured job/runtime group counts.

## Notes

