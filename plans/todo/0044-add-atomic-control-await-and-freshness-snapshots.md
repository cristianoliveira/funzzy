---
id: TASK-0044
title: Add atomic control await and freshness snapshots
status: todo
depends_on: [TASK-0021, TASK-0043]
priority: high
tags: [rust, cli, control-socket, await, freshness, json-rpc, tdd]
---

# Add atomic control await and freshness snapshots

## Problem
An agent must currently poll status and can race between filesystem events, scheduling, and completion, making it impossible to reliably await one relevant generation.

## Context

Add server method and CLI command that atomically observe current sequence then wait for terminal state. Implement condition/event notification; do not busy-poll.

## Acceptance criteria

- [ ] Tests first cover already-terminal, future completion, no generation yet, new batch during wait, superseded generation, watcher restart/disconnect, multiple waiters, and timeout boundary.
- [ ] `fzz control await [--after ID] [--generation ID] --timeout DURATION` has unambiguous validated modes.
- [ ] Server registration prevents snapshot-to-subscription lost-wakeup race.
- [ ] Response includes one consistent snapshot, terminal reason, latest observed batch/generation, pending debounce state, and freshness classification.
- [ ] `control run/emit --wait` reuse exact await primitive and return resulting observation in one round trip.
- [ ] Timeout bounds socket and server wait, performs no cancellation, and reports latest snapshot.
- [ ] Waiters do not block watcher scheduling or each other and are cleaned on client disconnect.
- [ ] Pi watcher client/domain decoder is updated additively.

## Notes

