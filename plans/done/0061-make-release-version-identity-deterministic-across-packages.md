---
id: TASK-0061
title: Make release version identity deterministic across packages
status: done
depends_on: [TASK-0060]
priority: high
tags: [rust, release, versioning, nix, ci, determinism, tdd]
---

# Make release version identity deterministic across packages

## Problem
Version identity is duplicated and already inconsistent across Cargo, lockfile, Nix stable package, tags, documentation, and protocol fixtures, allowing a release to publish different versions through different channels.

## Context

Treat `Cargo.toml` package version as source for built binary/crate. Tag and stable Nix package point to published immutable source and therefore need explicit consistency checks rather than fragile broad text replacement.

## Acceptance criteria

- [ ] Tests/check script inventory every version-bearing surface: Cargo.toml/lock, `fzz` and `funzzy --version`, capabilities `watcherVersion`, protocol fixtures, README release label, stable/local/nightly Nix packages, tag, and pi-watcher fixtures.
- [ ] One exact version command updates only intended release surfaces and supports `--check`/dry-run without modifying files.
- [ ] Cargo metadata is source of truth for binary/crate; no handwritten Rust package version is introduced.
- [ ] Check rejects Cargo/lock mismatch, tag without `v` + exact Cargo version, stale protocol fixture, and stable Nix version/revision mismatch.
- [ ] Local/nightly commit-hash versions remain separate from stable semantic version and are not accidentally rewritten.
- [ ] Stable Nix hash refresh is deterministic, target-specific failures are actionable, and script cannot leave placeholder hashes after failure.
- [ ] CI runs consistency check before publish and tag workflows verify checkout/tag/package equality before building/uploading.
- [ ] crates.io workflow cannot publish from arbitrary manual branch without exact release tag/version proof.
- [ ] Existing `v1.6.0`/Nix `1.5.0` inconsistency is documented and resolved by V2 candidate rather than silently rewriting historical tag.

## Notes

