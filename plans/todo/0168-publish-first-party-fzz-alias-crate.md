---
id: TASK-0168
title: Publish a first-party fzz alias crate
status: todo
depends_on: []
priority: high
tags: [security, supply-chain, crates-io, release, cli, packaging]
---

# Publish a first-party fzz alias crate

## Problem

The `funzzy` crate installs both `funzzy` and `fzz`, but crates.io does not currently contain a package named `fzz`. A user can reasonably try `cargo install fzz`. Until the project owns that package name, another publisher can create a misleading package and turn a predictable installation mistake into a supply-chain risk.

Verified on 2026-09-04:

- `https://crates.io/api/v1/crates/fzz` returns 404.
- `cargo info fzz` reports that the package does not exist.
- crates.io package `funzzy` is owned and published by `cristianoliveira`, points to this repository, and declares binary names `funzzy` and `fzz`.

## Desired outcome

The project owns a useful, first-party crates.io package named `fzz`. `cargo install fzz` installs the official short-name binary and runs the same application as `cargo install funzzy`.

## Acceptance criteria

- [ ] Confirm the crates.io name is still unclaimed immediately before any publish action; stop if ownership or metadata changed.
- [ ] Add a small publishable `fzz` package whose only purpose is the official Funzzy short-name installer; do not publish an empty placeholder.
- [ ] Make the alias package depend on an exact released `funzzy` version and call the existing public application entry point; do not duplicate application logic.
- [ ] Publish exactly one binary named `fzz`. Document that `funzzy` remains the canonical package and that both installation paths run the same program.
- [ ] Keep package metadata explicit: repository, license, README, description, keywords/categories, Rust version, and first-party ownership.
- [ ] Add deterministic version synchronization to the existing release/version check. Publish `funzzy` first and the matching `fzz` alias second.
- [ ] Test `cargo install --path` into a temporary root. Prove `fzz --version`, `fzz --help`, and representative parse/error behavior match the canonical binary.
- [ ] Run `cargo package --list` and `cargo publish --dry-run`; prove the package excludes plans, reports, sockets, logs, local configuration, and unrelated submodules.
- [ ] Update installation and release documentation only after the package exists. Explain that both `cargo install funzzy` and `cargo install fzz` are official.
- [ ] Prefer crates.io Trusted Publishing or another scoped release credential. Never print, persist, or commit a registry token.
- [ ] Require explicit human approval immediately before the irreversible first publish. Preparing, packaging, and dry-running do not authorize publication.
- [ ] After publish, verify crates.io owner, publisher, repository, checksum, version, and `bin_names`, then install from crates.io in a clean temporary Cargo root.
- [ ] Record recovery steps for partial release: if canonical `funzzy` publishes but alias publication fails, do not republish or overwrite; fix forward with the same compatible alias dependency or the next version.

## Security constraints

- Do not use a name-only or intentionally nonfunctional reservation package.
- Do not transfer ownership or add crates.io owners without explicit approval.
- Do not relax dependency checksums or use a Git dependency in the published alias.
- Do not make the alias package a second source of product behavior.
- Do not claim the namespace is protected until crates.io confirms the published package and intended owner.

## Non-goals

- Renaming the canonical `funzzy` crate.
- Removing the `fzz` binary from the canonical crate.
- Publishing a different tool under the short name.
- Changing CLI, YAML, control-socket, or runtime behavior.

## Delivery sequence

1. Prepare and test the alias package locally.
2. Review package contents and release automation.
3. Publish or confirm the matching canonical `funzzy` release.
4. Ask for explicit approval to publish `fzz`.
5. Publish once.
6. Verify registry metadata and clean installation.
7. Update public installation guidance and complete this plan.
