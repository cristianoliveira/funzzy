---
id: TASK-0013
title: Prove and document the Clap migration
status: todo
depends_on: [TASK-0012]
priority: normal
tags: [rust, cli, release]
---

# Prove and document the Clap migration

Issue: https://github.com/cristianoliveira/funzzy/issues/226

## Problem

Parser replacement is not releasable until generated help, packaging, both binaries, and feature-gated watcher behavior are proven together. Dependency changes may also invalidate Nix cargo hashes.

## Deliverable

Release-ready migration evidence and synchronized CLI/package documentation.

## Scope

- README/CLI docs only where Clap output or declared MSRV makes them stale
- Nix package cargo hashes/metadata required by new lockfile
- Full repository verification
- Issue/PR evidence summary

## Acceptance criteria

- [ ] Help documents all public commands/options and Funzzy environment variables.
- [ ] `funzzy` and `fzz` manual smoke cases pass for help, version, target-list, valid config, and invalid arguments.
- [ ] Nix local package builds with refreshed, deterministic dependency hash.
- [ ] No Docopt reference remains outside historical planning/release notes.
- [ ] No deprecated CLI aliases or compatibility parser path is introduced.
- [ ] Any intentional help/error presentation difference from Docopt is documented in PR evidence.
- [ ] Issue #226 acceptance is traceable to tests and verification commands.

## Verification

- `make lint`
- `make tests`
- `make integration`
- `make integration-e2e`
- `make nix-flake-check`
- `make nix-build-local`
- Manual smoke for both binary names
