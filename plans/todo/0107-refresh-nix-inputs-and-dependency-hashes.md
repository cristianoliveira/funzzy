---
id: TASK-0107
title: Refresh Nix inputs and dependency hashes
status: todo
depends_on: [TASK-0105, TASK-0106]
priority: high
tags: [nix, dependencies, packaging, cargo-hash, toolchains, reproducibility]
---

# Refresh Nix inputs and dependency hashes

## Problem
Cargo and npm dependency changes can leave Nix lock inputs, toolchains, and fixed-output cargo hashes stale, causing packaged builds to differ from local verification.

## Context

Refresh Nix only after Cargo and npm manifests stabilize so hashes are computed once. Root flake and pi-watcher flake have distinct nixpkgs pins and toolchain needs; do not collapse them without evidence.

## Acceptance criteria

- [ ] Record old/new flake input revisions, dates, and relevant Rust/Node package versions for root and pi-watcher.
- [ ] Update each flake input explicitly rather than opaque all-input churn; retain intentional nixpkgs channel differences with rationale.
- [ ] Ensure dev shell Rust satisfies declared MSRV 1.97 and CI/build compiler expectations; Node remains supported by pi-watcher engines and CI.
- [ ] Recompute root stable/local/nightly Cargo fixed-output hashes through existing bump scripts or reproducible Nix failure hashes—never hand-guess.
- [ ] Root `nix flake check`, stable/local/nightly package builds, and version consistency checks pass from clean source.
- [ ] pi-watcher `nix develop` can run `npm ci` and `make all`; lock input update does not substitute an unsupported Node major.
- [ ] Nix lockfiles and package hashes contain only expected dependency-input changes.
- [ ] Packaging excludes plans, reports, node_modules, sockets/logs, and submodule internals as before.
- [ ] Document any platform-specific build difference across default systems or explicitly narrow support rather than silently skipping it.

## Notes

Keep dependency identity reproducible across Cargo, npm, and Nix release channels.

