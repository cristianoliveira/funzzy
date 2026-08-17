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


## Prep complete (09-04-26, pre-publication)

- `scripts/verify-release` harness landed (`make verify-release`): per-channel verification — crates (isolated CARGO_HOME install, both binaries, init/check/list/run, watcher 130/143 signal contract), github (draft detection, 4 archives + sha256 verification, native binary run), nix (stable package version), compat (V1 config load, removed-flag rejection + hint gap surfaced, migrate atomicity), control (capabilities text facts: watcher/schema versions, status, clean TERM).
- `--fzz-bin PATH` rehearsal mode: compat+control channels rehearsed green against local v2.0.0 build; github/crates channels proven to fail honestly while v2.0.0 is unpublished (draft detection verified live).
- Removed-V1-flag hint gap CLOSED in-repo (pre-publication): `--non-block`/`-n` and `--target`/`-t` are hidden deprecated args rejected post-parse with targeted V2 replacement text (exit 2, stderr; see removed_flag_error in src/arguments.rs). Unit + black-box tests pin the hint; verify-release compat channel now requires it.
- Run once publication lands: `make verify-release` (default 2.0.0). Every criterion maps to a channel check.
- criterion 7 closed pre-publication: README install surface modernized — stale "unreleased on develop" banner replaced (v1 stays on the v1 branch + MIGRATION.md pointer); linux-install.sh rewritten for the v2 artifact contract (funzzy-vV-<target>.tar.gz names from on-release-bin.yml, x86_64/aarch64 detection, sha256 verification before install, PREFIX/BASE/FORCE_ARCH seams) with scripts/linux-install-test (4/4: happy path, tampered-checksum refusal, both arch mappings, fully offline via file:// fixtures). Old script 404s on release day (wrong archive names) — publication-blocking bug fixed in time.

## Hint gap closed (09-04-26, pre-publication)

criterion 4's "targeted V2 replacement hints" now a product behavior:
removed V1 flags (\u200b--non-block/-n, --target/-t) accepted as hidden
deprecated args, rejected with exact V2 replacement text + MIGRATION.md
pointer (stderr, exit 2 — same as any clap usage error). TDD: unit
(arguments.rs) + black-box (cli_arguments.rs) + verify-release compat
channel all assert the hint. Full gate gen=69 PASS.
