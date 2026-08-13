---
id: TASK-0017
title: Add explicit workflow list and target selection commands
status: doing
depends_on: [TASK-0015]
priority: high
tags: [rust, cli, workflow, targets, tdd]
---

# Add explicit workflow list and target selection commands

## Problem
Value-less --target overloads discovery and filtering, making configured tasks harder to discover and invocation ambiguous.

## Context

Replace value-less `--target` discovery with `fzz list`. Keep configured watch as default and allow `fzz watch TARGET` according to the matching contract from TASK-0014.

## Acceptance criteria

- [ ] Tests first cover empty config, valid targets, no matches, malformed config, custom config, and target/tag selection.
- [ ] `fzz list` prints stable task identity and enough trigger information to choose a target.
- [ ] `fzz watch TARGET` selects only intended tasks; no match is an actionable error.
- [ ] Plain `fzz` still watches all configured tasks.
- [ ] Value-less `--target` and empty-string sentinels are removed rather than retained as deprecated paths.
- [ ] Human output remains concise and deterministic.

## Notes

