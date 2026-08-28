---
id: TASK-0145
title: Migrate accepted nested and grouped configs through fzz migrate
status: todo
depends_on: [TASK-0144]
priority: normal
tags: [rust, cli, config, migration, v2, tdd]
---

# Migrate accepted nested and grouped configs through fzz migrate

## Problem

The production migrator cannot currently convert accepted shipped nested-group configs into valid preferred `jobs:` form, so examples cannot be migrated through the same safe path users are told to use.

## Evidence

`fzz -c <copy-of-examples/nested-groups.yml> migrate` fails safely: wrapping the root list leaves a nested `on/tasks` group as a `jobs` item, then production validation rejects `on` as an invalid job property. The original stays unchanged.

## Acceptance criteria

- [ ] Write failing migration tests first for nested groups, mixed regular jobs/groups, root grouped `tasks:`, empty groups, comments, multiline/quoted commands, tags, absolute patterns, and group/job pattern merging.
- [ ] Convert every accepted legacy shape supported by the parser into flat preferred ordered `jobs:` accepted by the production parser and `fzz check`.
- [ ] Materialize each nested job's effective change/ignore surface using the same merge/dedupe/order semantics as runtime parsing; ignore precedence and declaration order cannot change.
- [ ] Preserve job names, commands, quoting, run-on-init, cwd/env, service/output/recovery-compatible fields, and comments deterministically where representable.
- [ ] Keep simple root-list wrapping and root `tasks:` rename behavior compatible.
- [ ] Second migration is a byte-identical no-op; invalid/unsupported source fails actionably and is never partially rewritten.
- [ ] CLI replacement remains atomic and leaves original bytes untouched on parse, validation, or write failure.
- [ ] Do not broaden V2 parser shapes or restore nested groups under `jobs:`.

## Verification focus

Pure transform tests in `src/cli/migrate.rs`; black-box one shipped nested fixture through `fzz migrate` + `fzz check`.
