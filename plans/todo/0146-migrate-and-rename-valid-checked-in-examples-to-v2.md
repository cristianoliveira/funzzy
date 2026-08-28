---
id: TASK-0146
title: Migrate and rename valid checked-in examples to V2
status: todo
depends_on: [TASK-0145]
priority: normal
tags: [examples, config, v2, migration, tests]
---

# Migrate and rename valid checked-in examples to V2

## Problem

Users copying checked-in examples still learn deprecated configuration vocabulary, while tests and docs depend on current paths and behavior.

## Context

Apply only the approved TASK-0144 mapping and TASK-0145 production migration behavior. Already-V2 `recovery-format.yml` and `v2-parallel-control.yml` remain byte-identical unless the contract records a necessary canonical update.

## Acceptance criteria

- [ ] Migrate all 14 valid example configs to canonical V2 `jobs:` with no valid root list or `tasks:` key remaining.
- [ ] Use production `fzz migrate` for accepted legacy input; review every diff for order, commands, patterns, ignores, tags, init flags, and comments.
- [ ] Rename task-vocabulary paths according to TASK-0144 and update all Rust tests, docs, README links, fixture constants, and reload append logic atomically.
- [ ] Keep no duplicate legacy aliases or stale in-repo references.
- [ ] Preserve observable behavior for simple watch, verbose/log output, fail-fast, restart/non-block, reload, templates, tags/selection, absolute paths, run-on-init, and nested groups.
- [ ] Preserve relative paths against fixture workspaces and scratch absolute-path substitution seams.
- [ ] Ensure every migrated config passes `fzz check` and a second `fzz migrate` is a no-op.
- [ ] Keep legacy compatibility coverage in dedicated parser/migration tests rather than public examples.

## Non-goals

Do not change example commands/outcomes, add showcase fields, remove legacy parsing, or convert intentionally invalid fixtures (TASK-0147).
