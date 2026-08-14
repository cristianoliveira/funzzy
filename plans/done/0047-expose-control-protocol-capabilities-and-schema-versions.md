---
id: TASK-0047
title: Expose control protocol capabilities and schema versions
status: done
depends_on: [TASK-0021, TASK-0042]
priority: normal
tags: [rust, cli, control-socket, protocol, discovery, tdd]
---

# Expose control protocol capabilities and schema versions

## Problem
Agent clients cannot safely adapt to installed Funzzy versions without parsing help text or optimistically calling methods that may not exist.

## Context

Add cheap read-only `capabilities` method and CLI command. It reports protocol facts, not dynamic watcher state.

## Acceptance criteria

- [ ] Response includes protocol/schema version, watcher version, supported methods, optional fields/features, output formats, and limits relevant to clients.
- [ ] Ordering and representation are deterministic and compact.
- [ ] Method works while idle/busy and performs no config reload or filesystem scan.
- [ ] Older-server missing-method response is translated into actionable compatibility message.
- [ ] Client can gate await, emit, cancel, output retrieval, and structured schema without trial side effects.
- [ ] Versions follow documented compatibility rules rather than package version equality.
- [ ] Golden protocol and CLI tests prevent accidental schema drift.
- [ ] Pi watcher may consume capabilities but existing startup remains backward compatible.

## Notes

