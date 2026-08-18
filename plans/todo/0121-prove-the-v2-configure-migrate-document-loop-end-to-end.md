---
id: TASK-0121
title: Prove the V2 configure-migrate-document loop end to end
status: todo
depends_on: [TASK-0118, TASK-0119, TASK-0120]
priority: high
tags: [integration, config, migration, init, docs, v2, regression]
---

# Prove the V2 configure-migrate-document loop end to end

## Problem
Unit-level parity is insufficient unless fresh initialization, examples, validation, migration, execution, and documentation commands work together as users experience them.

## Context

This is the final regression and risk gate for the breaking configuration boundary.

## Acceptance criteria

- [ ] Add black-box coverage for `fzz init -> check -> list -> run/watch` using canonical V2.
- [ ] Add black-box coverage for every `fzz config example PROFILE -> check`, including agent control socket and parallel execution behavior.
- [ ] Preserve black-box coverage for `V1 root task list -> migrate -> jobs -> check -> list -> run/watch`, proving behavior and job order are unchanged.
- [ ] Prove migration still performs only root-list wrapping or `tasks:` renaming, existing `jobs:` remains byte-stable, and malformed/unsupported roots fail without mutation.
- [ ] Test success, failure, and close hooks from the new `hooks` section on happy and unhappy lifecycle paths.
- [ ] Test execution concurrency/output from the new `execution` section and event/socket behavior from `on`.
- [ ] Verify schema root and bounded sections describe every generated key and no unreachable pseudo-config sections remain.
- [ ] Add focused drift checks across option catalog, schema, all examples, `fzz init`, CLI help, and active documentation snippets.
- [ ] Run the configured watcher/final integration gate and record freshness evidence for the unchanged worktree fingerprint.
- [ ] Record compatibility and release-note impact, including the required user migration action.

## Notes

Success is one deterministic user loop, not merely green parser tests.
