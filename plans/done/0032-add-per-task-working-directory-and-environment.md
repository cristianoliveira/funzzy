---
id: TASK-0032
title: Add per-task working directory and environment
status: done
depends_on: [TASK-0025]
priority: high
tags: [rust, workflow, process, monorepo, config, tdd]
---

# Add per-task working directory and environment

## Problem
Configured tasks inherit one process root and environment, making monorepo workflows verbose and forcing commands to encode infrastructure setup themselves.

## Context

Add execution context to task plan, resolved from injected workspace root. Do not mutate global process directory/environment.

## Acceptance criteria

- [x] Parser tests cover optional `cwd` and string-to-string `env`, wrong types, empty names, and legacy configs.
- [x] Relative `cwd` resolves against config workspace root; absolute/path-escape policy is explicit.
- [x] Missing/non-directory `cwd` fails before command spawn with task-attributed error.
- [x] Task environment overlays inherited environment without leaking into siblings or later runs.
- [x] Template expansion uses task working directory where contract says it should.
- [x] Parallel tasks with distinct cwd/env execute independently.
- [x] Verbose/explain output shows effective cwd and environment keys while redacting values by default.
- [x] Integration tests cover paths with spaces and deterministic inheritance/removal semantics.

## Notes

