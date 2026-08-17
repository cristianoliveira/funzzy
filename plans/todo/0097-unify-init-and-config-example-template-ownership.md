---
id: TASK-0097
title: Unify init and config example template ownership
status: todo
depends_on: [TASK-0096]
priority: high
tags: [rust, cli, init, config, templates, tdd]
---

# Unify init and config example template ownership

## Problem
Init and config example intentionally differ by destination and audience, but independent generation paths can drift and make profile selection inconsistent.

## Context

Keep destination policy in command layer and generated YAML in one pure template/profile module. The comprehensive template may retain its catalog-driven commented layout, while named runnable profiles have exactly one renderer consumed by both commands.

## Acceptance criteria

- [ ] Write failing focused tests first for profile parity, create-only file behavior, stdout purity, and invalid template selection.
- [ ] Introduce one typed profile/template selector for `comprehensive`, `minimal`, `parallel`, and `agent`; Clap possible values and help derive from or are parity-tested against it.
- [ ] `fzz init` keeps comprehensive default and accepts `--template PROFILE` plus existing custom output filename behavior.
- [ ] `fzz init --template minimal|parallel|agent` writes bytes identical to `fzz config example PROFILE` stdout.
- [ ] `fzz config example` remains side-effect-free and emits only valid YAML to stdout; it never reads the project config or creates directories/files.
- [ ] Comprehensive output remains catalog-driven, deterministic, generic, immediately runnable, and bounded as defined by INIT-TEMPLATE-CONTRACT.
- [ ] Every generated artifact parses and validates through production parser; no command-specific copied YAML constants remain.
- [ ] Existing-file refusal occurs before rendering/writing and leaves original bytes unchanged for every profile.
- [ ] Parser/application dispatch models profile intent explicitly instead of encoding mutually exclusive optional fields.
- [ ] Focused unit and CLI argument tests cover happy and unhappy paths.

## Notes

Do not add `init --stdout`; `config example` already owns stdout generation.

