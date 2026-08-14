---
id: TASK-0060
title: Define the Funzzy v2.0.0 release boundary
status: done
depends_on: []
priority: high
tags: [release, semver, v2, compatibility, design]
---

# Define the Funzzy v2.0.0 release boundary

## Problem
Cargo already reports 1.6.0 while README describes an unreleased breaking V2 CLI, so release scope, semantic-version decision, compatibility promises, and go/no-go gates must be explicit before changing versions or tagging.

## Context

Target `2.0.0`: real subcommands and removed/renamed V1 flags are intentional public CLI breaks, so a minor bump would understate compatibility impact. Current latest tag and Cargo version are `v1.6.0`; stable Nix package still says `1.5.0`.

## Acceptance criteria

- [ ] Release decision records why next public version is `2.0.0`, not `1.7.0`, and distinguishes CLI/API break from additive control protocol evolution.
- [ ] Scope matrix names mandatory tasks, explicitly deferred tasks, and behavior that remains compatible (`funzzy`/`fzz`, zero-argument watch, legacy YAML, additive JSON-RPC fields).
- [ ] Go/no-go gate requires TASK-0020, TASK-0029, TASK-0049, TASK-0056, TASK-0059, packaging, migration, and security/license checks, or records explicit scope reduction before candidate cut.
- [ ] Supported Rust version, OS/architecture matrix, install channels, binaries, config formats, protocol/schema versions, and pi-watcher compatibility are declared.
- [ ] Version lifecycle defines candidate commit → dry-run → tag → GitHub release → crates publish → stable Nix update → post-publish verification.
- [ ] Release/tag ownership and exact manual approval boundary are documented; planning or CI cannot publish implicitly.
- [ ] Roll-forward policy is explicit: immutable tags/releases are never rewritten; defects produce `2.0.1` or withdrawn artifact with incident note.
- [ ] Release notes outline is approved before any version file changes.

## Notes

README already labels current documented CLI as unreleased V2.

