---
id: TASK-0096
title: Define init config and migration command responsibilities
status: todo
depends_on: []
priority: high
tags: [design, cli, init, config, migration, v2]
---

# Define init config and migration command responsibilities

## Problem
The CLI currently has overlapping configuration generators and places migration under initialization, so users cannot infer command behavior from command names alone.

## Context

Use familiar CLI semantics rather than minimizing command count:

- `init` creates a project configuration and therefore writes a file.
- `config schema|example` describes or exports the installed configuration contract and remains side-effect-free.
- migration transforms an existing configuration and must be named explicitly.

This refines TASK-0057/TASK-0058 and TASK-0093 without undoing their agent-discovery or comprehensive-starter goals. V2 is an intentional breaking boundary, so the contract does not need to preserve a deprecated alias.

## Acceptance criteria

- [ ] Update CLI V2, agent configuration, jobs configuration, init template, migration, and usage contracts with one responsibility table covering `init`, `config schema`, `config example`, and migration.
- [ ] Specify `fzz init` as create-only, file-writing behavior with deterministic existing-file refusal.
- [ ] Specify `fzz init --template comprehensive|minimal|parallel|agent`; default remains `comprehensive` so `fzz init && fzz` stays generic and runnable.
- [ ] Specify `fzz config schema|example` as stdout-only and side-effect-free; `config example PROFILE` remains the piping/agent surface and does not gain file-writing flags.
- [ ] Specify one explicit migration interface (`fzz migrate`, honoring global `-c/--config`) and remove `fzz init --migrate` from the V2 contract rather than carrying two paths.
- [ ] Define overwrite, output-channel, exit-code, deterministic-byte, invalid-profile, missing-file, malformed-file, and already-migrated behavior.
- [ ] Record why intentional generator overlap is acceptable: same profile artifact, different destination and user intent.
- [ ] Confirm no conflict with `fzz check`, which validates the selected project config rather than describing or creating it.

## Notes

Success means command names predict side effects before implementation starts.

