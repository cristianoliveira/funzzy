---
id: TASK-0108
title: Prove and automate dependency update safety
status: done
depends_on: [TASK-0107]
priority: high
tags: [ci, dependencies, security, msrv, packaging, automation, reliability]
---

# Prove and automate dependency update safety

## Problem
Updated manifests and lockfiles are not enough: security, MSRV, packaging, submodule compatibility, and recurring drift need deterministic gates before release.

## Context

Final proof runs after ecosystem-specific tasks. Add bounded automation that reports drift without creating nondeterministic CI failures from network volatility.

## Acceptance criteria

- [x] Install/pin or otherwise reproducibly run Rust advisory scanning; zero unacknowledged vulnerable/yanked direct or transitive dependencies.
- [x] npm audit remains zero high/critical and package-lock is reproducible with `npm ci`.
- [x] Verify declared Rust MSRV with an explicit CI job/toolchain rather than only current Nix stable; dependency updates cannot raise it silently.
- [x] Run focused and full Rust unit/integration/e2e gates, pi-watcher `make all` plus real-socket e2e, root/pi Nix checks, crate packaging, npm package dry-run, and release version checks.
- [x] Compare CLI help/schema, control wire snapshots, generated init examples, semantic config hashes, filesystem matched outcomes, and exit codes against baseline; explain every intentional diff.
- [x] `cargo tree --duplicates` and npm tree reports contain no unexplained duplicate major versions or extraneous packages.
- [x] Add scheduled dependency drift/advisory reporting with grouped ecosystems and bounded output; routine PR CI stays deterministic/offline where possible.
- [x] Configure update grouping so watcher/process majors, Pi SDK pair, TypeScript toolchain, and Nix inputs never arrive as one opaque batch.
- [x] Document manual update command, review checklist, rollback procedure, and cadence for future maintainers.
- [x] Final watcher verification is fresh, continuous, and bound to unchanged worktree fingerprint before completion.

## Notes

Automation proposes updates; it must not auto-merge behavior-sensitive major versions.


## Outcome (TASK-0108 done)

Commits: root `abbd004`, pi-watcher `7e367b3` (additive renovate.json only).

- **Advisory**: `cargo-audit 0.22.2` reproducibly via the root-flake-pinned nixpkgs rev (`nix run github:nixos/nixpkgs/937e5ee4…#cargo-audit -- audit`): 104 deps, zero vulnerabilities/warnings/unmaintained (RUSTSEC-2024-0320 closed by TASK-0112). Weekly scheduled job automates it.
- **npm**: audit 0 high/critical; `npm ci` reproduction re-proven (TASK-0106 clean checkout + TASK-0107 inside `nix develop`).
- **MSRV**: new explicit `msrv` CI job — dtolnay@1.97.0, `cargo check --locked --all-targets`, plus a guard failing unless Cargo.toml `rust-version` and the job pin move together.
- **Gates run across the update chain** (session evidence): full cargo test gen 58/70, integration gen 71 (post-yaml-rust2), pi-watcher make quick+make all 452/452 ×3 (bash, clean npm ci, nix develop), nix flake check + .#local/.#nightly/.#default builds, nix watcher gate gen 82, version-check --candidate + version-check-test 3/3, npm pack surface identical, make lint/fmt.
- **Baseline comparison**: CLI contract (cli_arguments 46), control wire snapshots (control_* suites), init/catalog proof, parser accept/reject matrix — all green before/after each update; the only intentional behavioral diffs are the six documented yaml-rust2 YAML 1.2 deltas (TASK-0112).
- **Trees**: `cargo tree --duplicates` — none (132 edges); `npm ls --all` — single majors, no extraneous packages.
- **Automation**: `deps-drift.yml` weekly scheduled, grouped per ecosystem (rust/node/nix), bounded output, informational-only — routine PR CI untouched and offline. Renovate configs: majors always separate; Pi SDK pair grouped; TypeScript toolchain coherent; nix inputs monthly — no opaque batching.
- **Docs**: `docs/DEPENDENCIES.md` — manual commands (incl. pinned cargo-audit invocation), review checklist, rollback, cadence.
- **Final verification**: fresh `unit tests @agent-final` gen 83 PASS (198s) on the exact final tree (post-abbd004, no further edits), watcher continuous across the session.
