---
id: TASK-0065
title: Audit V2 documentation and define one information architecture
status: done
depends_on: []
priority: high
tags: [docs, v2, information-architecture, audit, source-of-truth]
---

# Audit V2 documentation and define one information architecture

## Problem
Documentation mixes released V1 guidance, unreleased V2 behavior, normative implementation contracts, stale removed flags, and legacy examples, so readers and agents cannot tell which page is current executable truth.

## Context

Separate audiences and truth levels: README orientation, task-oriented guides, generated CLI/config reference, migration, troubleshooting, and normative contributor contracts. Do not expose planning/TASK language as primary user docs.

## Acceptance criteria

- [ ] Inventory classifies every README/docs/examples page as keep/rewrite/generate/archive/delete with owner, audience, source of truth, and V2 readiness.
- [ ] Audit records every stale command/config/version claim against current Clap help, parser, tests, and capabilities; known examples include `--non-block`, `--target`, old `-V/--verbose`, and V1.5/1.6 text.
- [ ] Proposed navigation has one obvious route for new users, configuration, daily commands, parallelism, control/agents, troubleshooting, migration, and reference.
- [ ] README is bounded orientation/install/quick-start/navigation, not duplicate full manual.
- [ ] Normative contracts remain contributor/reference evidence and are clearly labeled apart from supported user behavior.
- [ ] Generated sources are declared for command help, config schema/examples, protocol capability tables, and version strings; handwritten duplication is minimized.
- [ ] Versioning policy explains docs on develop versus tagged releases and stable URL strategy.
- [ ] Link/anchor conventions, terminology glossary, code-block execution policy, accessibility/style rules, and ownership are documented.
- [ ] Revamp map identifies release-blocking documentation versus deferred deep dives.

## Notes

Initial evidence: README says V2 unreleased while Cargo/tag are 1.6.0; `docs/USAGE.md`, `FLAG_NON_BLOCK.md`, `FLAG_TARGET.md`, `FLAG_CONTROL_SOCKET.md`, README, and example script still teach removed V1 vocabulary.

