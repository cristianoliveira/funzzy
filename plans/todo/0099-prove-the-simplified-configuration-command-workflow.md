---
id: TASK-0099
title: Prove the simplified configuration command workflow
status: todo
depends_on: [TASK-0097, TASK-0098]
priority: high
tags: [integration-tests, cli, init, config, migration, docs, reliability]
---

# Prove the simplified configuration command workflow

## Problem
Command restructuring can silently break agent discovery, initialization safety, migration fidelity, documentation, and the V2 release contract without black-box coverage.

## Context

Exercise installed binary in isolated temporary workspaces. Prefer behavioral assertions and focused snapshots over duplicating implementation strings.

## Acceptance criteria

- [ ] Black-box matrix proves default comprehensive init plus minimal, parallel, and agent templates create deterministic parser-valid `.watch.yaml` files.
- [ ] For each named profile, bytes written by `fzz init --template PROFILE` equal bytes printed by `fzz config example PROFILE`.
- [ ] `config schema|example` leave filesystem unchanged and print no prose around machine-consumable payloads.
- [ ] Init refuses an existing destination for every profile without modifying bytes; invalid profile is Clap usage error with valid-value correction.
- [ ] Migration matrix covers root list, grouped `tasks:`, preferred `jobs:`, malformed YAML, unsupported root, missing file, custom config path, comments, and newline variants.
- [ ] Failed migration leaves original bytes unchanged; successful migration passes `fzz check` and second migration is a byte-identical no-op.
- [ ] `fzz init --migrate` fails as unsupported usage and help advertises only create/template behavior.
- [ ] README and usage/migration guides present one path for create, export, inspect schema, validate, and migrate, with accurate side-effect table.
- [ ] Shell completions and CLI snapshots include `migrate` and init template values without stale `init --migrate` references.
- [ ] Agent configuration loop remains valid: discover schema/example, write or init, check, list/run/watch.
- [ ] Focused Rust tests, integration suite, lint, and documentation drift checks pass through configured watcher/final verification gate.

## Notes

This task is release-sensitive because TASK-0063/TASK-0084 are active; finish before publishing V2 or explicitly defer command changes to a later major boundary.

