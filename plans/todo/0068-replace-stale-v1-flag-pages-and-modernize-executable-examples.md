---
id: TASK-0068
title: Replace stale V1 flag pages and modernize executable examples
status: todo
depends_on: [TASK-0066, TASK-0067]
priority: high
tags: [docs, migration, examples, v1, v2, cleanup]
---

# Replace stale V1 flag pages and modernize executable examples

## Problem
Current docs and examples still teach removed --non-block and --target invocations and old configuration shapes, which will cause V2 users and agents to generate invalid commands.

## Context

Do not keep deprecated instructions alongside V2 as if both are valid. Preserve history through tagged V1 docs and explicit migration table, not live duplicate flag pages.

## Acceptance criteria

- [ ] `FLAG_NON_BLOCK.md` and `FLAG_TARGET.md` are removed/replaced by V2 wait/restart and target-selection pages with redirects/links only where site behavior supports them.
- [ ] Control, fail-fast, logging, init, usage, README, examples README, and scripts contain no stale removed invocation or incorrect short flag.
- [ ] Migration table maps every V1 command/flag/config shape to exact V2 replacement, behavior change, and exit-code impact.
- [ ] Examples use preferred grouped config and cover minimal, ignore/templates, cwd/env, tags, init, parallel, long-running restart, control socket, and agent-final target without redundant fixtures.
- [ ] Every valid example passes `fzz check`; intentionally invalid fixtures assert exact current diagnostics and are labeled.
- [ ] Example shell scripts use strict mode, bounded waits/timeouts, safe temp paths, and process cleanup.
- [ ] Obsolete/dead examples and duplicated prose are deleted rather than left unlinked.
- [ ] Repository-wide stale-vocabulary allowlist is minimal and limited to migration/history/contracts.
- [ ] Links from releases/tags to V1 documentation remain valid after live V2 cleanup.

## Notes

