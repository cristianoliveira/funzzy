---
id: TASK-0144
title: Define the shipped-examples V2 migration contract
status: todo
depends_on: []
priority: normal
tags: [examples, config, v2, migration, design]
---

# Define the shipped-examples V2 migration contract

## Problem

Checked-in examples still teach legacy root lists and `tasks:` syntax even though `jobs:` is the preferred V2 vocabulary, and their integration-fixture role makes unbounded rewriting risky.

## Context

Planning report: `.tmp/reports/28-08-26/examples-v2-migration-plan.md`.

Current inventory: 17 YAML configs — 14 valid (2 V2, 1 grouped `tasks:`, 11 root-list/nested legacy) and 3 intentionally invalid. TASK-0119 migrated generated init/example profiles, not this checked-in catalog.

## Acceptance criteria

- [ ] Classify every YAML file as already V2, directly migratable root/grouped legacy, nested legacy requiring semantic flattening, or intentionally invalid.
- [ ] Define an old→new filename map that removes active task vocabulary and update policy for all repository references; no duplicate legacy aliases.
- [ ] Define flattening semantics using production config behavior: preserve global job order, effective merged change/ignore patterns, ignore precedence, commands, tags, init flags, and execution context.
- [ ] Define a before/after behavior matrix for watch, init, list/explain/run, reload, fail-fast, restart, templates, tags, absolute paths, and nested groups.
- [ ] Define each invalid fixture's intended V2 failure reason so conversion cannot accidentally change what it proves.
- [ ] Separate public examples from dedicated legacy-compatibility fixtures; this task does not remove parser compatibility.
- [ ] Record deterministic, atomic, idempotent migration and recursive validation requirements for TASK-0145–0147.

## Non-goals

No production code or example rewrite. Do not redesign commands, runtime behavior, or unrelated inline test configs.
