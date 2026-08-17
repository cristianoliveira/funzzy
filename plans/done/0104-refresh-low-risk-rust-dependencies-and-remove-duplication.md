---
id: TASK-0104
title: Refresh low-risk Rust dependencies and remove duplication
status: done
depends_on: [TASK-0103]
priority: high
tags: [rust, cargo, dependencies, cleanup, tdd]
---

# Refresh low-risk Rust dependencies and remove duplication

## Problem
Several Rust dependencies are held behind current compatible releases or duplicated unnecessarily, increasing maintenance and supply-chain surface without requiring deliberate runtime behavior changes.

## Context

Handle non-watcher/process upgrades before high-risk API migrations. Keep each logical update in an independent commit with lockfile diff and focused verification.

## Acceptance criteria

- [ ] Record green focused baseline tests before editing each dependency family; add characterization tests only where public behavior lacks coverage.
- [ ] Refresh compatible direct/runtime and dev dependencies to policy-approved stable versions, including serde/json, clap/clap_complete, glob, ignore, predicates, pretty_assertions, and assert_cmd.
- [ ] Evaluate `serde_derive` as separate direct dependency versus `serde` derive feature; choose one source without changing wire representations.
- [ ] Replace `once_cell::Lazy` with standard `LazyLock` only if MSRV 1.97 guarantees equivalent initialization semantics, then remove unused crate.
- [ ] Upgrade SHA-2 only with deterministic semantic-hash fixtures proving config revision identity does not change unexpectedly; defer major if output compatibility cannot be preserved.
- [ ] Remove any direct dependency proven unused by `cargo tree` plus source search.
- [ ] Keep `Cargo.toml` formatting and version constraint style consistent and explain intentional exact pins.
- [ ] `cargo tree --duplicates` has no new duplicate major families; every remaining duplicate is documented.
- [ ] Unit, CLI argument, config revision, schema, and default-feature tests pass after each update group.
- [ ] Cargo.lock changes contain only expected packages and checksums.

## Notes

Do not combine notify/nix migration here; TASK-0105 owns behavioral dependencies.

