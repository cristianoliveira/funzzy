---
id: TASK-0025
title: Preserve task boundaries in run plans and outcomes
status: done
depends_on: [TASK-0024]
priority: high
tags: [rust, execution, domain, refactor, tdd]
---

# Preserve task boundaries in run plans and outcomes

## Problem
Commands are flattened before execution, losing task identity needed to run independent tasks concurrently and aggregate results by workflow task.

## Context

Introduce task-aware values such as `RunPlan`, `SerialStage`/`ParallelGroup`, `TaskPlan`, `RunOutcome`, and `TaskOutcome` in execution domain. Names are illustrative; use user-facing workflow vocabulary. Configuration parser must preserve named contiguous group occurrences and barriers instead of returning only flat `Vec<Rules>`.

## Acceptance criteria

- [x] Pure tests first preserve group name/occurrence, barrier topology, task name, config position, commands, trigger, expanded path values, and unknown template variables.
- [x] Parser accepts `parallel: <group-name>` and rejects boolean, numeric, collection, and empty values with task-local actionable errors.
- [x] Legacy tasks without `parallel` parse unchanged and retain sequential behavior.
- [x] Parser accepts optional positive integer `on.concurrency`, defaults through injected available-parallelism provider, and rejects zero/wrong types deterministically.
- [x] Parsed workflow becomes ordered serial tasks and named parallel-group occurrences instead of flat `Vec<Rules>` or `Vec<String>`.
- [x] Each task plan retains sequential command order and stable task identity/path within plan.
- [x] Path/target filtering removes unmatched tasks without merging group occurrences across original barriers; empty stages disappear safely.
- [x] Outcomes represent passed, failed, cancelled, and skipped tasks plus per-command failures (durations land with execution timing in TASK-0026).
- [x] Overall run outcome derives deterministically from task outcomes.
- [x] Planning/template expansion has no process, stdout, control-socket, or threading side effects.
- [x] Existing sequential behavior can execute same plan with concurrency limit one.

## Notes

