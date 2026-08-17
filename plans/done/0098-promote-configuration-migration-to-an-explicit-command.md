---
id: TASK-0098
title: Promote configuration migration to an explicit command
status: done
depends_on: [TASK-0096]
priority: high
tags: [rust, cli, config, migration, jobs, tdd]
---

# Promote configuration migration to an explicit command

## Problem
Migrating an existing file is a transformation, not project initialization, and fzz init --migrate obscures that destructive operation.

## Context

Move existing migration policy rather than rewriting it. Parsing and deterministic transformation belong outside init command; CLI adapter selects configured path and reports outcome. This is a V2 breaking cleanup, so do not retain `init --migrate` as a deprecated path.

## Acceptance criteria

- [ ] Write failing parser and command tests first for `fzz migrate`, removed `fzz init --migrate`, custom `-c/--config`, malformed input, and missing input.
- [ ] Add top-level `fzz migrate` with help that states it rewrites existing accepted legacy configuration into preferred ordered `jobs:` form.
- [ ] Honor global config selection exactly as `check`, `list`, and watch do; default remains `.watch.yaml`.
- [ ] Remove migrate state from `InitCommand`, `Arguments`, and init dispatch so initialization has one create-only path.
- [ ] Preserve root-list wrapping, grouped `tasks:` rename, declaration order, comments, quoting, commands, and trailing-newline behavior.
- [ ] Preserve deterministic idempotence for already preferred `jobs:` input without rewriting bytes.
- [ ] Migration validates the complete candidate before replacement and never leaves partial/truncated config after any error.
- [ ] Success and failure messages name selected file and stay on correct output channel; exit codes remain 0 success and 1 operational/config failure.
- [ ] Migration transformation is independently unit-testable without filesystem or stdout coupling.
- [ ] Remove all active `init --migrate` help and examples rather than leaving hidden compatibility behavior.

## Notes

Keep legacy config parser compatibility unchanged; this task only changes explicit rewrite entrypoint.

