---
id: TASK-0093
title: Define complete commented init template contract
status: done
depends_on: [TASK-0033, TASK-0057, TASK-0075]
priority: high
tags: [design, cli, init, config, documentation, determinism]
---

# Define complete commented init template contract

## Problem
fzz init currently creates a small runnable file that omits most supported .watch.yaml settings, forcing users to search docs and allowing parser schema examples and generated configuration to drift.

## Context

Follow TypeScript's generated `tsconfig.json` pattern: small active setup plus commented discoverable options. Preserve today's generic try-it-now experience; do not generate Cargo/npm/project-specific commands.

## Acceptance criteria

- [ ] Contract inventories every supported preferred `.watch.yaml` root, `on`, and `jobs[]` property from production parser; CLI-only controls and legacy `tasks:` inputs are explicitly excluded.
- [ ] Generated file remains immediately runnable in any directory: active generic hello job runs on init and active generic file-change job demonstrates matching without requiring a language/toolchain.
- [ ] Active example stays small and behaviorally equivalent to today's `echo`/`ls` starter rather than activating every option.
- [ ] Every optional supported property appears commented near owning section with one brief purpose, default when meaningful, allowed values, and shape/example where ambiguity exists.
- [ ] Required properties and inheritance semantics (`on` shared patterns, job extension, declaration order, contiguous parallel groups) are explained without turning file into full manual.
- [ ] Template documents next commands `fzz check`, `fzz list`, and `fzz`/`fzz watch`, plus installed authoritative `fzz config schema` discovery.
- [ ] Preferred ordered `jobs:` only; legacy formats and migration prose stay out of generated file.
- [ ] Comments never include secret-like values and environment example uses harmless placeholder.
- [ ] Deterministic size/readability budget and stable ordering are defined; no terminal width, environment, repository, or network dependency.
- [ ] `fzz config example minimal` remains concise machine-copyable alternative; no new `fzz init --minimal` flag without separate evidence.

## Notes

Success means user can run `fzz init && fzz` immediately, while same file acts as bounded configuration index.
