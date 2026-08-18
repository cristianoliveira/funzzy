---
id: TASK-0118
title: Preserve migration as V1 task-list to V2 jobs only
status: done
depends_on: [TASK-0117]
priority: high
tags: [rust, cli, config, migration, v1, v2, regression]
---

# Preserve migration as V1 task-list to V2 jobs only

## Problem
Reorganizing the V2 configuration could accidentally turn `fzz migrate` into a general configuration upgrader, but today it has one narrow responsibility: converting V1 task vocabulary to V2 ordered `jobs`.

## Context

Current `migrate_content` performs exactly these transformations:

1. V1 root task list -> wrap under `jobs:`.
2. Accepted grouped `tasks:` -> rename root key to `jobs:`.
3. Existing `jobs:` -> byte-identical no-op.

It preserves comments, quoting, commands, order, and trailing-newline behavior, validates the candidate, and replaces atomically. The new `on`/`execution`/`hooks` organization must not expand this transformation set.

## Acceptance criteria

- [x] Keep the pure migration transformation limited to root task-list wrapping and root `tasks:` to `jobs:` renaming.
- [x] Do not move `on.concurrency`, `on.output`, or hook properties; do not use migration to reorganize any V2 section.
- [x] Keep existing `jobs:` input a byte-identical no-op.
- [x] Preserve comments, declaration order, quoting, commands, and trailing-newline behavior exactly as current tests require.
- [x] Preserve complete-candidate validation, configured path handling, staging, atomic replacement, deterministic output, and failure safety.
- [x] Keep malformed YAML, unsupported roots, and mixed `tasks`/`jobs` behavior unchanged unless the existing contract test exposes a defect.
- [x] Add a focused regression proving the V2 section reorganization does not change `migrate_content` output for representative V1 input.
- [x] Keep CLI help and success messages scoped to migration into the preferred `jobs:` form.
- [x] Do not describe or implement migration for old grouped V2 field placement, future config shapes, formatting, or cleanup.

## Outcome

`migrate_content` remains a pure task-vocabulary transformation. The regression pins old grouped policy text byte-for-byte, including its original `on.*` placement.

## Notes

This is a preservation task, not a migration redesign. If the V2 section change needs compatibility handling, it belongs in parser policy or release documentation—not `fzz migrate`.
