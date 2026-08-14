---
id: TASK-0039
title: Export structured run events and outcomes
status: done
depends_on: [TASK-0028, TASK-0043]
priority: normal
tags: [rust, output, ndjson, agents, editors, protocol, tdd]
---

# Export structured run events and outcomes

## Problem
Agents and editor integrations must parse decorated human stdout because Funzzy has no bounded machine-readable stream for task lifecycle and final outcomes.

## Context

Prefer NDJSON event stream plus final outcome over parsing human logs. Reuse executor event model and version schema.

## Acceptance criteria

- [ ] Contract defines destination, framing, schema version, event kinds, IDs, timestamps/durations, and exit semantics.
- [ ] Every event includes run generation; task events include stable task/group occurrence identity.
- [ ] Concurrent events remain valid line-delimited records without byte interleaving.
- [ ] Final record contains complete order-independent combined outcome.
- [ ] Human stderr/stdout and machine stream cannot corrupt each other.
- [ ] Output is bounded/streamed and handles broken consumer pipe predictably.
- [ ] Golden tests cover pass/fail/cancel/supersede and schema compatibility.
- [ ] Control protocol reuses semantic types without forcing same transport shape.

## Notes

