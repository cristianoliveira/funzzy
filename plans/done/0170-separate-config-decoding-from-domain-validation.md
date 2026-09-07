---
id: TASK-0170
title: Separate configuration decoding from domain validation
status: done
depends_on: [TASK-0169]
priority: high
tags: [architecture, config, domain, validation]
---

# Separate configuration decoding from domain validation

## Problem

`src/config.rs` combines YAML shape decoding, defaults, cross-field validation, legacy compatibility, and filesystem-facing errors. This makes domain rules depend on parser and infrastructure concerns and makes changes risky.

## Desired outcome

Decode YAML at the edge into neutral input data, then validate and construct domain configuration through pure functions that depend only on domain types and explicit ports.

## Acceptance criteria

- [x] Split `from_yaml`/`rule_from_with_common` responsibilities into edge decoding and pure domain validation without changing accepted legacy/task-list formats. `RuleInput` and `build_rule` now receive only decoded domain values.
- [x] Keep filesystem/path existence checks in an adapter or application boundary; domain validation remains filesystem-independent. Existing `config.rs` path/file helpers remain edge adapters.
- [x] Preserve defaults, error categories/order where public, glob semantics, readiness, recovery, and reload candidate policy. Full config and reload suites are green; explicit ordering characterization covers cross-field vs later output errors.
- [x] Add tests proving domain validation runs with no filesystem, CLI, watcher, process, control, or stdout dependencies. `config_validation` imports only `rules` and std; static boundary guard covers it.
- [x] Add happy/unhappy tests for preferred and legacy YAML, cross-field errors, and reload candidates.
- [x] Re-run module graph and confirm config domain code does not import CLI or runtime modules. Graph shows `config -> config_validation -> rules` and no reverse/runtime edge.

## Verification

- `cargo test --lib config::`: 126 passed.
- `cargo test --lib config_validation::`: 8 passed.
- `cargo test --lib reload::`: 16 passed.
- `cargo test --test domain_boundaries`: 8 passed.
- Fresh watcher current-HEAD gates: unit generation 102, integration generation 103, format generation 104; all passed with fingerprint `9ae1c747ce47`.
- `make lint`: passed.
- Generated config/schema and existing CLI/watcher/control/process behavior are covered by the full unit and feature-gated integration gates.
- Detailed responsibility/evidence report: `.tmp/reports/04-09-26/task-0170-config-seam.md`.
