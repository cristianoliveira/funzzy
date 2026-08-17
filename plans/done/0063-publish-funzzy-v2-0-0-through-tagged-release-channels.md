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
