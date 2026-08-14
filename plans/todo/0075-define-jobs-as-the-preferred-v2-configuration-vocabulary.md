---
id: TASK-0075
title: Define jobs as the preferred V2 configuration vocabulary
status: todo
depends_on: [TASK-0014, TASK-0065]
priority: high
tags: [design, config, jobs, migration, ubiquitous-language, v2]
---

# Define jobs as the preferred V2 configuration vocabulary

## Problem
The preferred grouped config still uses tasks while users commonly recognize jobs as independently executable workflow units, but copying GitHub Actions shape blindly would conflict with Funzzy ordered barriers and runtime task identities.

## Context

Recommended V2 shape renames root `tasks:` list to ordered `jobs:` list:

```yaml
on:
  change: "src/**"
  concurrency: 2
jobs:
  - name: lint
    parallel: checks
    run: cargo clippy
```

Do not copy GitHub Actions mapping/`steps` model: Funzzy declaration order and contiguous barriers are semantic, while GitHub jobs are unordered DAG nodes. Define configured **job** versus runtime **task execution** explicitly.

## Acceptance criteria

- [ ] Contract explains jobs as configured workflow units, tasks as per-generation runtime executions, commands as sequential work inside job.
- [ ] Preferred root shape is ordered `jobs: [ ... ]`; mapping-form jobs are rejected with actionable list example to preserve deterministic order.
- [ ] Existing `on`, matching, ignore, name/tag, run, cwd/env, init, parallel, hooks/policies semantics remain unchanged except vocabulary.
- [ ] Compatibility decision for existing root list and grouped `tasks:` is explicit; V2 emits only jobs and migration path is deterministic.
- [ ] Mixed `tasks` + `jobs`, duplicate names, empty jobs, and scalar/mapping forms have locked errors/precedence with no silent merge.
- [ ] Parallel group names/barrier occurrences remain based on declaration order; job rename cannot imply dependency DAG or automatic independence.
- [ ] Config schema version, CLI/config migration, user diagnostics, control protocol, and duration-signature effects are defined.
- [ ] JSON-RPC `tasks` identity remains additive compatibility field for runtime task executions unless separate protocol revision explicitly changes it.
- [ ] `on.concurrency` remains scheduler bound; root `jobs` does not introduce `on.jobs` alias or ambiguity.

## Notes

This is a V2 preferred configuration refactor, not an attempt to clone GitHub Actions.

