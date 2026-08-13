---
id: TASK-0040
title: Add workflow success and failure hooks
status: todo
depends_on: [TASK-0027]
priority: normal
tags: [rust, workflow, hooks, automation, tdd]
---

# Add workflow success and failure hooks

## Problem
Users need notifications and follow-up automation, but platform-specific built-ins would couple Funzzy to desktops and browsers; generic terminal-outcome hooks offer a composable surface.

## Context

Keep hooks generic and finite. Avoid built-in desktop/browser integrations. Hook-induced filesystem events must remain observable for loop diagnosis.

## Acceptance criteria

- [ ] Contract defines run-level versus task/group hooks, terminal outcomes, ordering, environment/templates, and whether hook failure changes outcome.
- [ ] Parser rejects recursive hooks and ambiguous/incompatible declarations.
- [ ] Hooks use same process runner/context and are cancellable/reaped.
- [ ] Exactly one applicable terminal hook runs per generation, including superseded/cancelled policy.
- [ ] Hook events carry generation/correlation and appear in verbose/structured output.
- [ ] Tests cover pass, fail, cancel, hook failure, restart race, and possible feedback loop.
- [ ] Examples show notification composition without platform-specific dependency.

## Notes

