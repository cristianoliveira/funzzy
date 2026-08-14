---
id: TASK-0076
title: Parse and emit ordered jobs configuration
status: todo
depends_on: [TASK-0075, TASK-0005]
priority: high
tags: [rust, config, parser, init, migration, jobs, tdd]
---

# Parse and emit ordered jobs configuration

## Problem
A vocabulary contract has no effect until config loading, init, migration, schema, examples, and validation treat root jobs as preferred while preserving deterministic declaration order and explicit compatibility behavior.

## Context

Keep one internal parsed job model and adapters for explicitly accepted legacy forms; do not duplicate watch/run planning paths.

## Acceptance criteria

- [ ] Parser tests first cover preferred jobs list, grouped tasks compatibility, legacy root list, mixed keys, mapping/scalar/null jobs, empty list, duplicate names, comments, and declaration order.
- [ ] Preferred `jobs:` flows through same matching, filtering, topology, context resolution, execution, output, and control target paths as current tasks.
- [ ] `fzz init` emits only preferred ordered jobs format; `init --migrate` converts accepted tasks forms while preserving comments, commands, names, order, barriers, and matching.
- [ ] Migration is idempotent and atomic, creates explicit backup/diff policy, and never starts watcher or tasks.
- [ ] Config schema/example source identifies jobs as preferred and tasks as compatibility/deprecated input according to TASK-0075.
- [ ] Errors name exact YAML path and provide copyable ordered-list correction; jobs mapping is never reordered implicitly.
- [ ] Config reload changes tasks→jobs spelling without duplicate generation or changed execution signature when semantics are identical.
- [ ] Runtime job model remains separate from persistence/control/task outcome compatibility serialization.
- [ ] Current examples/fixtures are migrated in focused slices without broad unreviewed formatting changes.

## Notes

