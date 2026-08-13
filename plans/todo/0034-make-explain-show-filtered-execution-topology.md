---
id: TASK-0034
title: Make explain show filtered execution topology
status: todo
depends_on: [TASK-0019, TASK-0025, TASK-0033]
priority: high
tags: [rust, cli, explain, workflow, diagnostics, tdd]
---

# Make explain show filtered execution topology

## Problem
Listing which tasks match a path is insufficient once barriers and named parallel groups exist; users need to see the actual filtered execution plan before running it.

## Context

Extend planned path explanation from match list into actual run-plan preview after target/path/init filtering.

## Acceptance criteria

- [ ] Pure tests cover serial plan, parallel groups, ignored path, group separator filtered out, repeated group names, and empty plan.
- [ ] Output distinguishes matched, ignored, selected, and skipped tasks with effective rule origin.
- [ ] Displays barriers and named group occurrences without implying completion order.
- [ ] Shows effective jobs, debounce, cwd, and busy/failure policies relevant to plan.
- [ ] Uses same matcher/planner as execution and never spawns work.
- [ ] Human output is concise; structured representation is stable if machine format exists.
- [ ] Invalid config delegates to same validation diagnostics as `fzz check`.

## Notes

