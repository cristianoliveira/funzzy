---
id: TASK-0041
title: Add task-aware output policies
status: done
depends_on: [TASK-0028]
priority: normal
tags: [rust, output, workflow, logs, tdd]
---

# Add task-aware output policies

## Problem
Parallel workflows and long-running watches need control over noise and failures, but output behavior is currently global and cannot capture, suppress, or reveal output by task outcome.

## Context

Define small composable output policy rather than many booleans. Keep streaming default and memory bounded.

## Acceptance criteria

- [ ] Contract evaluates `inherit`, `quiet`, `capture`, and `show-on-failure` against existing stdout/stderr/log behavior.
- [ ] Parser rejects invalid policies while legacy tasks retain current output.
- [ ] Parallel output remains line-atomic and task attributed under every policy.
- [ ] Captured output has explicit byte bound and truncation marker; large output cannot exhaust memory.
- [ ] Log file and machine export receive documented data once without duplication.
- [ ] Failure/cancellation reveals correct buffered output exactly once.
- [ ] Tests cover partial lines, stderr, non-UTF8 policy, truncation, broken pipe, and mixed sibling policies.

## Notes

