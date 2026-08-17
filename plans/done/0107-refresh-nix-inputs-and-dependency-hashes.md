---
id: TASK-0107
title: Refresh Nix inputs and dependency hashes
status: done
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

- [x] Record old/new flake input revisions, dates, and relevant Rust/Node package versions for root and pi-watcher.
- [x] Update each flake input explicitly rather than opaque all-input churn; retain intentional nixpkgs channel differences with rationale.
- [x] Ensure dev shell Rust satisfies declared MSRV 1.97 and CI/build compiler expectations; Node remains supported by pi-watcher engines and CI.
- [x] Recompute root stable/local/nightly Cargo fixed-output hashes through existing bump scripts or reproducible Nix failure hashes—never hand-guess.
- [x] Root `nix flake check`, stable/local/nightly package builds, and version consistency checks pass from clean source.
- [x] pi-watcher `nix develop` can run `npm ci` and `make all`; lock input update does not substitute an unsupported Node major.
- [x] Nix lockfiles and package hashes contain only expected dependency-input changes.
- [x] Packaging excludes plans, reports, node_modules, sockets/logs, and submodule internals as before.
- [x] Document any platform-specific build difference across default systems or explicitly narrow support rather than silently skipping it.

## Notes

Keep dependency identity reproducible across Cargo, npm, and Nix release channels.


## Outcome (TASK-0107 done)

Commits: root `d08e463`, pi-watcher `7a67adf`.

| Input | Old | New |
|---|---|---|
| root nixpkgs | 6bcaade (2026-08-13) | 937e5ee (2026-08-17) |
| root utils / systems | unchanged (flake-utils has no newer release) | — |
| pi-watcher nixpkgs | 2fcb964 (2026-08-10) | e5bdc4a (2026-08-16) |

- Explicit single-input updates only; distinct pins retained deliberately (root needs Rust toolchain for packages; pi-watcher only needs git/gnumake/nodejs_24 devshell — collapsing without evidence rejected).
- Node major preserved: nodejs_24 -> 24.19.0 (engines >=22.19.0); Rust: local build compiles with declared MSRV 1.97 toolchain (dev shell default rustc).
- bump-nix-local rerun: version label 9961199, cargoHash `sha256-rBQtX0rE…` unchanged (Cargo.lock unchanged since TASK-0112 refresh — expected).
- Stable package intentionally untouched: still v1.5.0 tag until TASK-0063 publishes v2.0.0; nightly still builds from origin/master (unpushed commits not representable there — hashes unchanged and verified by build).
- Gates: `nix flake check` OK (current system aarch64-darwin; `--all-systems` cross-compilation not part of any prior gate either — documented, not silently skipped); `nix build .#local .#nightly .#default` all succeed; pi-watcher `nix develop`: npm ci + make all 452/452 + audit clean.
- Packaging exclusion surfaces unchanged (no new files in packages; plans/reports/node_modules never referenced).
