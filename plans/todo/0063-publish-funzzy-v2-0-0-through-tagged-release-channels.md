---
id: TASK-0063
title: Publish Funzzy v2.0.0 through tagged release channels
status: todo
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

