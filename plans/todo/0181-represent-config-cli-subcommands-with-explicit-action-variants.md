---
id: TASK-0181
title: Represent config CLI subcommands with explicit action variants
status: todo
depends_on: []
priority: normal
tags: [rust, cli, types, readability]
---

# Represent config CLI subcommands with explicit action variants

## Problem
Action::Config encodes schema and example commands as combinations of optional fields, including Option<Option<String>>, allowing ambiguous or contradictory states.

## Context

`src/arguments.rs::Action::Config` uses `schema_section: Option<Option<String>>` and `example_profile: Option<String>`. TASK-0172 thinned application dispatch, but left this ambiguous semantic action shape in place.

## Scope and approach

- Introduce an explicit `ConfigAction` with `Schema { section, format }` and `Example { profile, format }` variants.
- Extract config-subcommand match decoding into a focused helper; let dispatch match the semantic variant.
- Inspect public Rust consumers before changing exported `Action`. Preserve its current shape via an internal normalized action/facade if compatibility requires it; do not silently break callers.
- Retain Clap ownership of help, usage errors, and argument validation.

## Acceptance criteria

- [ ] Internal config dispatch represents exactly one command; contradictory optional-field combinations do not flow through use-case execution.
- [ ] Tests cover schema with/without section, example profiles, supported formats, and invalid section/profile/format or missing arguments according to the existing contract.
- [ ] Help text, stdout/stderr channels, exit codes, default format/profile, and unrelated CLI precedence remain unchanged.
- [ ] Public Rust API compatibility is assessed and either preserved or an explicit breaking decision is requested before implementation.
- [ ] No configuration file reads or watcher startup are introduced into schema/example commands.

## Verification

Characterize parser/dispatch behavior first. Run focused arguments and config-command tests plus relevant CLI/agent-config integration tests. Use the fresh watcher final gate and retain domain-boundary checks.

## Likely files

`src/arguments.rs`, `src/app.rs`, `src/cli/config.rs` if required, and corresponding tests.

## Non-goals

No CLI grammar redesign, new flags, new output formats, or global `Action` rewrite.
