---
id: TASK-0036
title: Respect gitignore rules with explainable precedence
status: done
depends_on: [TASK-0033]
priority: normal
tags: [rust, watcher, gitignore, matching, diagnostics, tdd]
---

# Respect gitignore rules with explainable precedence

## Problem
Repository and generated files commonly create watcher noise; manually duplicating every ignore rule in Funzzy config is error-prone and can cause feedback loops.

## Context

Use established ignore semantics/library rather than partial parser. Decide explicit default and override so compatibility is not surprising.

## Acceptance criteria

- [ ] Contract decides default, `respect_gitignore`, nested ignore files, global excludes, negation, and explicit watch override precedence.
- [ ] Matching tests cover nested repositories, anchored rules, negation, ignored directories, symlinks, and paths outside root.
- [ ] Existing explicit `ignore` remains strongest or precedence is migration-documented.
- [ ] Explain output names exact ignore source file/rule.
- [ ] Ignore cache reloads when relevant file changes without event-loss gap.
- [ ] Behavior remains deterministic across equivalent relative/absolute paths.
- [ ] Performance test avoids rescanning all ignore files per event.

## Notes

