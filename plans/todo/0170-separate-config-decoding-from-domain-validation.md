---
id: TASK-0170
title: Separate configuration decoding from domain validation
status: todo
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

- [ ] Split `from_yaml`/`rule_from_with_common` responsibilities into edge decoding and pure domain validation without changing accepted legacy/task-list formats.
- [ ] Keep filesystem/path existence checks in an adapter or application boundary; domain validation remains filesystem-independent.
- [ ] Preserve defaults, error categories/order where public, glob semantics, readiness, recovery, and reload candidate policy.
- [ ] Add tests proving domain validation runs with no filesystem, CLI, watcher, process, control, or stdout dependencies.
- [ ] Add happy/unhappy tests for preferred and legacy YAML, cross-field errors, and reload candidates.
- [ ] Re-run module graph and confirm config domain code does not import CLI or runtime modules.

## Verification

Run config unit tests, generated-config tests, reload/config integration tests, `cargo test --lib`, feature-gated integration tests, and `make lint`. Compare SOLID/complexity results with the baseline.
