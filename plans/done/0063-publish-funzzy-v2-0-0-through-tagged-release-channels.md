---
id: TASK-0063
title: Publish Funzzy v2.0.0 through tagged release channels
status: done
depends_on: [TASK-0062, TASK-0084]
priority: high
tags: [release, github, crates-io, nix, supply-chain]
---

# Publish Funzzy v2.0.0 through tagged release channels

## Problem
The verified candidate must be tagged and published exactly once across GitHub artifacts and crates.io without tag/package version mismatch or accidental publication from a dirty or moving commit.

## Context

This task contains irreversible remote writes. Require explicit human approval of exact candidate SHA and release notes immediately before execution. The earlier candidate is superseded by reproduced output AXI defects; publication must use refreshed TASK-0084 candidate and new approval.

## Acceptance criteria

- [ ] Publisher confirms authenticated identities, required secrets, candidate SHA, clean tree, `2.0.0` metadata, and absence of existing `v2.0.0` tag/crate before mutation.
- [ ] Annotated/signed `v2.0.0` tag points exactly to approved candidate and is pushed once; tag is never moved or recreated.
- [ ] GitHub release uses approved notes and is created as draft first when platform flow permits.
- [ ] Multi-architecture workflow uploads expected Linux/Darwin x86_64/aarch64 archives with checksums and both binaries.
- [ ] crates.io publication originates from exact tag and published crate metadata/version match tag.
- [ ] Stable Nix package points to `v2.0.0` with correct source/cargo hashes and reproducibly builds before merge/default-channel promotion.
- [ ] Workflow URLs, artifact checksums, crate URL, release URL, tag SHA, and stable Nix evidence are recorded.
- [ ] Partial failure stops remaining dependent publication and records safe resume/roll-forward action without republishing completed channel.
- [ ] No force push, tag rewrite, release overwrite, or `cargo yank` occurs without separate explicit incident decision.

## Notes


## Approval packet (prepared 09-04-26, user chose HOLD — nothing executed)

- Candidate SHA: `b20f12236c986fc3a3c936c040b3c71e1e6d7324` (master HEAD; supersedes TASK-0084-era `84d526b` per task notes).
- Pre-flight ALL GREEN: gh auth (cristianoliveira, repo scope); CRATES_TOKEN secret present; crates.io funzzy@2.0.0 untaken (max 1.5.0); remote tag v2.0.0 absent; `version-check --candidate` OK; `cargo publish --dry-run` clean; multi-arch workflow verified — checksum step was missing and landed as `b20f122` (sha256 files beside every archive).
- Approved-pending: delete stale LOCAL-ONLY v2.0.0 tag (never pushed; points at superseded 84d526b), re-tag annotated at candidate, push once.
- Release notes draft: `.tmp/reports/09-04-26/0063-release-notes-draft.md`.
- Execution order on approval: push master -> tag+push -> (auto) 4-arch artifacts w/ checksums -> GitHub release draft -> publish (triggers on-release crates.io from exact tag after identity gate) -> stable Nix bump to v2.0.0 + reproducible build -> record URLs/checksums -> TASK-0064.
- Failure policy: stop remaining channels on partial failure, record resume point, no republish of completed channels; no force-push/tag-rewrite/release-overwrite/yank without separate incident decision.

## Closure (09-04-26)

Ownership of the irreversible remote writes transferred to Cristian ("I'll do
it myself"). Publication was NOT executed from this session and, as of this
timestamp, no channel (remote tag / GitHub release / crates.io 2.0.0) carried
the release yet. Everything needed is in place for a one-shot manual run:

- Approved candidate: `b20f122` (checksums fix included; pre-flight table + execution order + failure policy in the packet above).
- Release notes draft: `.tmp/reports/09-04-26/0063-release-notes-draft.md`.
- Reminder for the manual run: delete the stale LOCAL `v2.0.0` tag first
  (points at superseded `84d526b`); push master before tagging.

TASK-0064 (post-publication verification) is unblocked by this closure but
cannot start until the channels actually carry v2.0.0.

## Addendum (09-04-26, later session)

master advanced past packet candidate b20f122 with pre-publication polish
that the release verification (TASK-0064 criterion 4) requires: removed-V1
flag targeted hints (hidden deprecated args + post-parse rejection). For the
manual run, tag the CURRENT master HEAD after pushing (packet SHA b20f122 is
superseded by this later commit); all other packet steps unchanged.

## Release notes (final draft — embedded 09-04-26 so the manual run needs no .tmp file)

Paste-ready body for the GitHub release and crates.io notes:

# Funzzy v2.0.0 — Release notes (DRAFT for approval)

Candidate: `b20f12236c986fc3a3c936c040b3c71e1e6d7324`

---

## Funzzy 2.0 — the agent-native file watcher

A ground-up rebuild of the CLI and execution model, designed for both humans
and coding agents. Same binaries (`funzzy` / `fzz`), same `.watch.yaml`
project contract — now with a real subcommand CLI, bounded parallel
execution, and a machine-readable control protocol.

### Breaking changes from 1.x

- **New CLI**: real subcommands (`watch`, `run`, `list`, `check`, `init`,
  `config`, `migrate`, `explain`, `control`/`ctl`) with conventional global
  flags. Removed V1 flags: `--non-block` (→ `on-busy: wait|restart` policy),
  `--target` (→ `watch TARGET` / `list`). Run `fzz migrate` to upgrade
  configs; `fzz explain` shows exactly which jobs match a path.
- **MSRV 1.97** (from 1.74): required by the modern dependency line
  (Clap 4, notify 8, yaml-rust2).
- Config: `jobs:` is the preferred vocabulary; legacy root-list and grouped
  `tasks:` forms remain accepted and migrate byte-preservingly.
- Parser upgraded to YAML 1.2 semantics (yaml-rust2): duplicate keys are now
  rejected instead of silently last-wins; block scalars keep their trailing
  newline; tab-indented blocks are rejected.

### Highlights

- **Bounded parallel execution**: independent jobs run concurrently with
  configurable limits, per-job working directory and environment, output
  attribution, and `parallel:` groups for ordering. Sequential override for
  diagnosis.
- **Control socket (agents)**: JSON-RPC over a Unix socket — status,
  targets, run/await, cancellation, paged output retention per generation,
  freshness snapshots, lifecycle subscriptions, and TOON/JSON output
  (`fzz control ...`). Powers the `pi-watcher` Pi extension.
- **Config hot reload**: valid changes swap watch roots and policy without
  process exit; invalid changes fail closed.
- **Reliability**: process-group ownership with graceful shutdown, close and
  success/failure hooks, gitignore-aware matching with explainable
  precedence, polling fallback, watching for files that don't exist yet.
- **Duration estimates**: historical run times produce deterministic
  estimates per target.
- **`fzz init`**: comprehensive commented starter generated from the option
  catalog — runnable immediately, documents every supported field.
- **Docs**: rewritten onboarding, configuration, daily and advanced guides;
  agent contracts for config discovery, feedback, and output evidence.

### Install

Prebuilt archives with sha256 checksums are attached to this release
(Linux/Darwin, x86_64/aarch64, both binaries). Nix: the stable package moves
to this tag shortly after publication. `cargo install funzzy --version 2.0.0`.

### Upgrading from 1.x

`fzz migrate` rewrites accepted legacy configs to the `jobs:` form
(atomically, byte-preserving comments/quoting/order; idempotent). See
`docs/MIGRATION.md` for the full flag/config mapping.

---

## Missing channel found: Homebrew tap (09-04-26)

README promises `brew install funzzy` and `brew install cristianoliveira/tap/funzzy`. The tap formula
(cristianoliveira/homebrew-tap, `funzzy.rb`) still points at v1.5.0 with the OLD artifact naming
(`funzzy-v1.5.0-x86_64-apple-darwin.tar.gz` — v2 produces `funzzy-v2.0.0-x86_64-darwin.tar.gz`).
Post-publish step (needs published sha256s, cannot be pre-computed):

1. Update `funzzy.rb`: version v2.0.0; arm64 + intel darwin URLs using v2 names (`funzzy-v2.0.0-aarch64-darwin.tar.gz` / `funzzy-v2.0.0-x86_64-darwin.tar.gz`) with `on_arm`/`on_intel` blocks; both sha256s from the release assets. `bin.install` for `funzzy` and `fzz` already correct.
2. Then verify criterion: `brew install cristianoliveira/tap/funzzy && fzz --version == 2.0.0`.

Add to the release record alongside tag/crate/Nix evidence.

## Final readiness (09-04-26, remote-verified)

master fully pushed through `0fdd3ed` (includes installer fix `ced38c5`, V1 hints `28c0bb0`, verify harness). Remote CI on `0fdd3ed`: CI Checks (build/lint/unit + first run of new MSRV job) ✓, nix build ✓, integration tests ✓. The manual run is now exactly:

1. delete stale LOCAL v2.0.0 tag; `git tag -a v2.0.0 0fdd3ed` (annotated, notes from the embedded draft); `git push origin v2.0.0`
2. artifacts + sha256 build automatically (on-release-bin); publish the existing draft release with the embedded notes
3. crates.io publishes from the exact tag (on-release identity gate)
4. tap + stable Nix per the packet steps above; then `make verify-release`
