---
id: TASK-0062
title: Prepare the v2.0.0 release candidate
status: todo
depends_on: [TASK-0061, TASK-0020, TASK-0029, TASK-0049, TASK-0056, TASK-0059]
priority: high
tags: [release, v2, migration, packaging, verification]
---

# Prepare the v2.0.0 release candidate

## Problem
A version edit alone is not a releasable artifact; the completed CLI, parallel, agent-feedback, duration-estimate, and configuration-discovery work needs migration notes, release evidence, packaging checks, and one immutable candidate commit.

## Context

Candidate preparation is reversible and makes no remote release/tag write. It produces one clean commit whose exact SHA is approved for publication.

## Acceptance criteria

- [ ] Version command changes Cargo.toml/Cargo.lock and intended V2 documentation/fixtures to exactly `2.0.0`; both binaries and capabilities report it.
- [ ] Release notes enumerate breaking CLI grammar/flags/exit behavior, preserved zero-argument mode/config compatibility, parallel groups, control socket, agent feedback, duration estimates, and configuration discovery.
- [ ] Migration table gives copyable V1 → V2 commands and config guidance; removed paths are not described as still supported.
- [ ] `cargo publish --dry-run` succeeds from packaged crate contents and excludes local state, reports, plans, sockets, logs, and secrets.
- [ ] GitHub archive/Nix local builds produce both `funzzy` and `fzz`; archives use deterministic names and contain license/readme as declared.
- [ ] Focused, integration, release, Nix flake/build, minimum-Rust, and pi-watcher real-server compatibility gates pass on clean candidate fingerprint.
- [ ] Dependency/license/security checks are recorded with explicit accepted exceptions.
- [ ] Release notes and candidate SHA are reviewed; repository is clean except known ignored build outputs.
- [ ] No tag, GitHub release, crates publish, or stable-channel mutation occurs in this task.

## Notes

