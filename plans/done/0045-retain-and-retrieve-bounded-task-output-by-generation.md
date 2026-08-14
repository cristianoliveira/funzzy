---
id: TASK-0045
title: Retain and retrieve bounded task output by generation
status: done
depends_on: [TASK-0028, TASK-0044]
priority: high
tags: [rust, control-socket, output, diagnostics, memory, tdd]
---

# Retain and retrieve bounded task output by generation

## Problem
Compact outcomes need actionable evidence, but returning full command logs exhausts context while returning only exit status forces agents to reproduce failures manually.

## Context

Store bounded output per generation/task separately from live forwarding. Default status/await includes concise failure excerpt; `control output` retrieves detail.

## Acceptance criteria

- [x] Contract fixes per-stream/per-task byte bound, generation retention count or TTL, truncation marker, and eviction behavior.
- [x] Tests first cover stdout/stderr, partial lines, non-UTF8 policy, huge output, concurrent tasks, cancellation, eviction, missing generation/task, and watcher restart.
- [x] `fzz control output --generation ID [--task ID] [--stdout|--stderr] [--tail N|--full]` remains bounded even for `--full` by declared retained limit.
- [x] Failure outcome includes small deterministic diagnostic excerpt, total retained/observed size, truncation, and exact retrieval hint.
- [x] Capture does not duplicate log-file/live output and cannot deadlock child pipes.
- [x] Secrets are not inferred; docs state command output may contain secrets and socket permissions are security boundary.
- [x] Memory use is globally bounded across generations and tasks with deterministic eviction.
- [x] Structured and human renderers consume same retrieval domain result.

## Notes

