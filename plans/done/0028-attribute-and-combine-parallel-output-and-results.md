---
id: TASK-0028
title: Attribute and combine parallel output and results
status: done
depends_on: [TASK-0027, TASK-0023, TASK-0043]
priority: high
tags: [rust, concurrency, output, control-socket, diagnostics, tdd]
---

# Attribute and combine parallel output and results

## Problem
Concurrent completion and child output have no stable order. Users do not require artificial ordering, but every line and outcome must retain task identity and combine into correct run result regardless of completion order.

## Context

Completion order inside named parallel group is intentionally nondeterministic and not part of contract. Stream useful feedback while preserving group/task ownership. Combine outcomes with order-independent reduction keyed by task identity; do not buffer merely to recreate config order.

## Acceptance criteria

- [ ] Tests first cover interleaved stdout/stderr, partial lines, binary/non-UTF8 handling policy, all outcome combinations, and out-of-order completion.
- [ ] Live child output is line-atomic and attributed to task (and command when needed), without byte-level corruption.
- [ ] Final task summary identifies group and every task; ordering inside parallel group is explicitly unspecified and tests compare task-keyed outcomes rather than sequence.
- [ ] Combined run state and exit result are derived once with order-independent reduction from structured outcomes.
- [ ] Group barrier summary is emitted only after all selected tasks in named group occurrence are terminal.
- [ ] `--log-file` preserves same attribution and does not duplicate forwarded output.
- [ ] Verbose records correlate task lifecycle with run generation using TASK-0023 vocabulary.
- [ ] Existing control status fields remain backward compatible; optional per-task detail is additive, compact, and coordinated with Pi watcher decoders/tests.
- [ ] Output buffering is bounded or streamed; implementation cannot accumulate unlimited child output in memory.

## Notes

