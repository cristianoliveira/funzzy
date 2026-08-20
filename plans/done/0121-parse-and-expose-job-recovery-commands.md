---
id: TASK-0121
title: Parse and expose job recovery commands
status: done
depends_on: [TASK-0120]
priority: high
tags: [rust, config, schema, jobs, recovery, tdd]
---

# Parse and expose job recovery commands

## Problem
The preferred configuration, runtime job model, schema, and diagnostics cannot represent a user-approved recovery command or command sequence on one job.

## Context

Implement the configuration/model portion of the TASK-0120 contract before execution changes. Keep `recovery` in the job domain and `recovery_policy` in execution policy; do not overload generation-level `hooks.failure`.

```yaml
execution:
  recovery_policy: prompt

jobs:
  - name: format-check
    run: cargo fmt --all -- --check
    recovery:
      - cargo fmt --all
      - git diff --check
```

## Acceptance criteria

- [x] Write failing parser and model tests first for scalar recovery, ordered recovery list, missing recovery, invalid type, empty scalar/list, non-string list member, unknown sibling property, forbidden service+recovery combination, and valid/invalid `execution.recovery_policy` values.
- [x] Add `recovery` and `execution.recovery_policy` to the canonical option catalog so parser allowlists, generated JSON Schema, bounded schema sections, `fzz check`, init/examples, and validation errors share one property definition.
- [x] Parse `recovery_policy` as an explicit execution-policy enum with `prompt` default and `skip` alternative; reject booleans and aliases such as `auto`, `always`, or `never`.
- [x] Store recovery commands as a private job-domain value with explicit accessors/builders; do not leak raw YAML into `Rules` or execution code.
- [x] Preserve declared recovery command order and distinguish an absent recovery from an invalid/empty recovery.
- [x] Expand recovery commands through the existing command/template model using the same job trigger context, resolved cwd, and environment as the original commands.
- [x] Carry expanded recovery commands into `TaskPlan` without changing ordinary jobs, ad-hoc `exec`, legacy root task-list behavior, or generation hooks.
- [x] Include recovery command identity in config semantic revision hashes and target execution signatures without persisting or exposing command/environment content in profile keys.
- [x] Ensure verbose formatting, `list`/`explain`, schema examples, and actionable errors make recovery eligibility discoverable without claiming that a recovery was approved.
- [x] Reject `recovery` in unsupported legacy/mixed shapes according to TASK-0120's compatibility decision; do not silently reinterpret it as a hook.
- [x] Cover happy and unhappy paths with focused deterministic unit tests before implementation.

## Outcome

Implemented the recovery configuration/model boundary across `config`, `Rules`, `TaskPlan`, schema/catalog, diagnostics, reload revisions, and execution signatures. Recovery commands remain declarative; no recovery command is executed by this task.

## Notes

No command may execute in this task. It only establishes a coherent configuration and planning boundary for TASK-0122.
