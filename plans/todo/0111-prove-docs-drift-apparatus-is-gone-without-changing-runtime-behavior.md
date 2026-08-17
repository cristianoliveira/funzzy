---
id: TASK-0111
title: Prove docs drift apparatus is gone without changing runtime behavior
status: todo
depends_on: [TASK-0110, TASK-0061]
priority: high
tags: [tests, docs, cleanup, ci, regression]
---

# Prove docs drift apparatus is gone without changing runtime behavior

## Problem
Removal must be complete and must not accidentally remove version release checks or alter fzz init config parsing CLI watcher and packaging behavior.

## Context

Verification is a one-time removal audit plus existing normal project gates, not a new permanent docs checker.

## Acceptance criteria

- [ ] Repository search finds no live `docs-drift`, `docs-drift-check`, `make docs-drift`, generic `DRIFT:` diagnostic, or `scripts/golden/init-template.yaml` reference outside historical plans/reports.
- [ ] `scripts/` retains only independently justified version, Nix bump, and Git hook helpers; no generated-doc/check replacement is introduced.
- [ ] Make help and GitHub workflow YAML parse and contain no dangling/renamed docs gate.
- [ ] Existing focused tests prove `fzz init` creates valid generic try-it-now config, refuses overwrite, handles custom filename/migration, and production parser/schema behavior remains unchanged.
- [ ] `scripts/version-check-test` still passes and release workflow retains Cargo/Cargo.lock/exact-tag checks.
- [ ] Unit, focused init/config tests, formatting, and ordinary CI checks pass without invoking removed script.
- [ ] No test reads a pre-existing `target/debug/fzz` as documentation truth or requires deleted golden path.
- [ ] Git diff confirms no runtime config, watcher, control protocol, CLI argument, schema, or init-template content changed.
- [ ] Removal report lists deleted checks plainly without claiming equivalent replacement coverage.
- [ ] External watcher final gate passes from unchanged worktree fingerprint.

## Notes
Renumbered from duplicate TASK-0096/0097/0098 (collided with init/config stream from f1e08e2) to restore unique IDs; deps updated in kind.
