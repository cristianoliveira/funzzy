---
id: TASK-0002
title: Unify Rust library and binary composition roots
status: todo
depends_on: []
priority: high
tags: [rust, architecture, cli]
---

# Unify Rust library and binary composition roots

## Problem

`src/lib.rs` and `src/main.rs` independently declare the same modules. Integration tests exercise library types while executable behavior compiles through a second module root, increasing wiring drift and shotgun-surgery risk.

## Scope

- `src/lib.rs`
- `src/main.rs`
- CLI application entry point and related tests

## Acceptance criteria

- [ ] `lib.rs` is canonical module root.
- [ ] `main.rs` is a thin process adapter that calls library application behavior.
- [ ] `funzzy` and `fzz` preserve flags, environment variables, output, and exit behavior.
- [ ] New module wiring has one obvious place.
- [ ] Integration tests execute same application modules used by binaries.
- [ ] No deprecated compatibility path is introduced.

## Verification

- CLI happy and error paths pass for both binary names.
- Default and feature-gated integration suites pass.

