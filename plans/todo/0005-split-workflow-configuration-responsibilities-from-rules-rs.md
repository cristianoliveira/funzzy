---
id: TASK-0005
title: Split workflow configuration responsibilities from rules.rs
status: todo
depends_on: []
priority: normal
tags: [rust, workflow, cohesion]
---

# Split workflow configuration responsibilities from rules.rs

## Problem

`src/rules.rs` combines task model, YAML parsing, compatibility formats, validation, glob matching, command templates, file loading, control socket extraction, and presentation. Its 1,829 lines make configuration changes high-risk.

## Scope

Incremental extraction around stable domain concepts; no configuration-format break.

## Acceptance criteria

- [ ] Tests characterize legacy list, grouped `on/tasks`, nested groups, merging, and invalid input before moves.
- [ ] Pure command template expansion is isolated from YAML and stdout side effects.
- [ ] User-facing task model is separated from parser DTO/YAML retention.
- [ ] Glob matching and validation have focused module ownership.
- [ ] Legacy and current YAML remain accepted exactly as documented.
- [ ] Public APIs are renamed only with impact checks and one-way migration.

## Verification

- Unit tests cover each extracted boundary's happy and unhappy paths.
- All config and filesystem integration tests pass.

