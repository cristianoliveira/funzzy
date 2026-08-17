---
id: TASK-0108
title: Prove and automate dependency update safety
status: todo
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

- [ ] Install/pin or otherwise reproducibly run Rust advisory scanning; zero unacknowledged vulnerable/yanked direct or transitive dependencies.
- [ ] npm audit remains zero high/critical and package-lock is reproducible with `npm ci`.
- [ ] Verify declared Rust MSRV with an explicit CI job/toolchain rather than only current Nix stable; dependency updates cannot raise it silently.
- [ ] Run focused and full Rust unit/integration/e2e gates, pi-watcher `make all` plus real-socket e2e, root/pi Nix checks, crate packaging, npm package dry-run, and release version checks.
- [ ] Compare CLI help/schema, control wire snapshots, generated init examples, semantic config hashes, filesystem matched outcomes, and exit codes against baseline; explain every intentional diff.
- [ ] `cargo tree --duplicates` and npm tree reports contain no unexplained duplicate major versions or extraneous packages.
- [ ] Add scheduled dependency drift/advisory reporting with grouped ecosystems and bounded output; routine PR CI stays deterministic/offline where possible.
- [ ] Configure update grouping so watcher/process majors, Pi SDK pair, TypeScript toolchain, and Nix inputs never arrive as one opaque batch.
- [ ] Document manual update command, review checklist, rollback procedure, and cadence for future maintainers.
- [ ] Final watcher verification is fresh, continuous, and bound to unchanged worktree fingerprint before completion.

## Notes

Automation proposes updates; it must not auto-merge behavior-sensitive major versions.

