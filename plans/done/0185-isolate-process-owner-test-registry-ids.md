---
id: TASK-0185
title: Isolate process-owner test registry IDs
status: done
depends_on: []
priority: normal
tags: [tests, determinism]
---

# Isolate process-owner test registry IDs

## Problem

`register_then_unregister_forgets_the_group` and `register_is_idempotent` share registry ID `999_991`. Parallel unit tests can unregister the ID between another test's registration and assertion, making the tests interfere nondeterministically.

## Approach

Give each test its own impossible process-group ID while preserving explicit cleanup and all production registry behavior.

## Acceptance criteria

- [x] The two registry tests use distinct test-local IDs.
- [x] Explicit unregister cleanup remains in each test.
- [x] No production code or sleeps change.
- [x] Repeated process-owner library tests pass.
- [x] Nix build/check passes when available.

## Evidence

- `cargo test --lib process_owner -- --test-threads=8`: 30/30 runs passed.
- `cargo fmt --all -- --check`: passed.
- `make nix-flake-check`: passed.
- `make nix-build-local`: passed; Nix unit check reported 917 passed, 0 failed.
