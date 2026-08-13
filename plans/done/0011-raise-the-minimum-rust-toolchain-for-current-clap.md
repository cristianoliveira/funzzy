---
id: TASK-0011
title: Raise the minimum Rust toolchain for current Clap
status: done
depends_on: []
priority: high
tags: [rust, nix, dependencies]
---

# Raise the minimum Rust toolchain for current Clap

Issue: https://github.com/cristianoliveira/funzzy/issues/226

## Problem

Repository currently builds with Rust/Cargo 1.78. Current Clap 4.6 resolves edition-2024 components requiring Rust 1.85, so Cargo 1.78 cannot even parse their manifests. Pinning old Clap internals would preserve hidden toolchain debt.

## Deliverable

One explicit, deterministic Rust 1.85-or-newer baseline across Cargo metadata and existing Nix development/build paths.

## Scope

- `Cargo.toml` minimum Rust declaration
- Existing Nix pin and development/package tooling
- CI/toolchain documentation only where needed

## Acceptance criteria

- [x] `Cargo.toml` declares `rust-version = "1.85"` or a justified newer minimum.
- [x] `nix develop` and Nix package builds provide compiler satisfying declared minimum.
- [x] Repository keeps one obvious Nix-owned toolchain path; no redundant toolchain manager is added.
- [x] CI uses compiler satisfying declared minimum and does not depend on runner accident.
- [x] Nix lock/hash changes are limited to toolchain requirement.
- [x] Existing exact `assert_cmd` pin is not opportunistically changed in this deliverable.
- [x] Rust upgrade alone does not change Funzzy runtime or CLI behavior.

## Verification

- `rustc --version` and `nix develop -c rustc --version` satisfy declared minimum.
- `cargo check --locked`
- `make lint`
- `make tests`
- `make nix-flake-check`
