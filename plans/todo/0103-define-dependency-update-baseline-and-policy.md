---
id: TASK-0103
title: Define dependency update baseline and policy
status: todo
depends_on: []
priority: high
tags: [design, dependencies, rust, node, nix, security, release]
---

# Define dependency update baseline and policy

## Problem
Funzzy spans Cargo, npm, a Git submodule, and two Nix flakes, but there is no single inventory or update policy to separate safe refreshes from behavior-changing major upgrades.

## Context

Inventory root `Cargo.toml`/`Cargo.lock`, `pi-watcher/package.json`/`package-lock.json`, root and extension flakes, Nix cargo hashes, CI toolchains, and the pi-watcher gitlink. Capture exact before-state without modifying lockfiles.

Current evidence:

- Cargo lock is current within declared constraints, but six direct constraints are behind newer releases.
- Direct `notify 4` coexists with `notify 6` through `notify-debouncer-mini 0.3`; current stable lines are notify 8 and debouncer 0.7.
- `nix 0.26` is behind 0.31 and is signal/process critical.
- pi-watcher has Pi SDK 0.84.2 and TypeBox 1.3.15 updates available; TypeScript reports a major 7.x update.
- npm audit currently reports zero vulnerabilities; cargo-audit is not installed.

## Acceptance criteria

- [ ] Produce one checked-in dependency inventory grouped by runtime, development, peer, transitive duplicate, Nix input, and toolchain dependency.
- [ ] Record current, declared, latest stable, license, maintenance status, advisory status, and source for every direct dependency.
- [ ] Distinguish compatible lock refresh, manifest minor update, major/API migration, replacement/removal, and explicit deferral.
- [ ] Define success gates: Rust MSRV 1.97, Node 24 extension support, both lockfiles deterministic, no high/critical advisories, no unexplained duplicate major versions, and reproducible Nix builds.
- [ ] Baseline focused unit, integration, pi-watcher `make all`, package contents, and Nix checks before upgrades; record existing unrelated failures rather than normalizing them.
- [ ] Decide whether `yaml-rust` remains intentionally frozen or migrates to maintained alternative in separate behavior task; do not silently swap parsers during bulk update.
- [ ] Decide whether TypeScript 7 is supported by current Pi SDK/ESLint/Vitest stack; defer with reason if ecosystem compatibility is not established.
- [ ] Define update ordering and rollback points so each dependency family is reviewable and bisectable.
- [ ] Account for pi-watcher as separate repository/submodule: extension changes land and verify there before root gitlink update.

## Notes

Do not run a blind `cargo update`, `npm update`, or `nix flake update` as implementation strategy.

