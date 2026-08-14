---
id: TASK-0064
title: Verify v2.0.0 installation and compatibility after publication
status: todo
depends_on: [TASK-0063]
priority: high
tags: [release, smoke-test, compatibility, pi-watcher, rollback]
---

# Verify v2.0.0 installation and compatibility after publication

## Problem
Successful release workflows do not prove users can install both binaries from each supported channel or that pi-watcher negotiates the published protocol correctly.

## Context

Verification runs against downloaded/published artifacts in fresh isolated environments, not local target directory or candidate checkout.

## Acceptance criteria

- [ ] Fresh crates.io install reports `2.0.0` for `funzzy` and `fzz`, creates config, lists/runs target, and starts/stops watch safely.
- [ ] Every GitHub archive checksum matches and native representative binaries report `2.0.0`; archive names/contents match release notes.
- [ ] Stable Nix install reports `2.0.0` and both aliases resolve to same package behavior.
- [ ] Minimal V1 config still loads or migration error matches declared contract; breaking V1 CLI commands produce targeted V2 replacement hints.
- [ ] Control capabilities report expected watcher/protocol/schema/features and current pi-watcher negotiates both advanced and legacy fallback paths.
- [ ] Agent flow discovers config schema/example, validates config, verifies target, retrieves bounded failure evidence, and cancels exact generation.
- [ ] README install commands resolve published release rather than develop/nightly source.
- [ ] Post-publish evidence and known limitations are attached to release record.
- [ ] Any critical failure opens explicit `2.0.1` roll-forward plan; immutable `v2.0.0` history remains intact.

## Notes

