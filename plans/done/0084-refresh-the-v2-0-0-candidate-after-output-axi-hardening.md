---
id: TASK-0084
title: Refresh the v2.0.0 candidate after output AXI hardening
status: done
depends_on: [TASK-0083, TASK-0087, TASK-0092, TASK-0095, TASK-0062]
priority: high
tags: [release, v2, candidate, axi, pi-watcher, verification]
---

# Refresh the v2.0.0 candidate after output AXI hardening

## Problem
The existing candidate predates a reproduced agent-facing output loop and cannot be published as exact approved SHA until server, pi-watcher fixtures, docs, package contents, and verification evidence include the hardening.

## Context

No remote write. Re-run candidate preparation from clean exact SHA after server and pi-watcher one-hop evidence work lands; prior candidate approval/evidence cannot be reused.

## Acceptance criteria

- [ ] Funzzy output hardening TASK-0079–0083, future-file watcher coverage TASK-0085–0087, valid-hot/invalid-fatal config reload TASK-0088–0092, complete init reference TASK-0093–0095, and corresponding pi-watcher TASK-0022–0025 commits/fixtures are present in clean parent/submodule candidate.
- [ ] Protocol/version compatibility decision is recorded; crate/docs/schema/help/package fixtures describe exact shipped output contract.
- [ ] Focused Rust and pi-watcher real-server E2E prove one-hop retrieval and transport bound; full release gates pass from unchanged fingerprint.
- [ ] Candidate report names exact SHA, submodule SHA, worktree cleanliness, watcher generation, artifacts, and differences from superseded candidate.
- [ ] Release notes call out exact output references, typed errors, paging/bounds, and compatibility/reload requirements accurately.
- [ ] Prior approval is explicitly invalidated; TASK-0063 requests new exact SHA approval immediately before any tag/publish write.
- [ ] No tag, release, crates.io publication, Nix promotion, or remote mutation occurs in this task.

## Notes
