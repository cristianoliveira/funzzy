---
id: TASK-0019
title: Explain which workflows match a path
status: done
depends_on: [TASK-0017]
priority: normal
tags: [rust, cli, workflow, diagnostics, tdd]
---

# Explain which workflows match a path

## Problem
Users cannot easily diagnose why a file change runs or skips a configured task, especially with merged change and ignore rules.

## Context

`fzz explain PATH` should reuse rule matching policy; output code must not reimplement matching decisions.

## Acceptance criteria

- [x] Tests first cover matched, ignored, unmatched, absolute, relative, and invalid paths.
- [x] Output identifies each selected task and matching change rule.
- [x] Output identifies ignore rules that win over change matches.
- [x] Grouped/common rule merging is reflected accurately.
- [x] Results are deterministic and command performs no watch or task execution.
- [x] No-match output is informative rather than silent.

## Notes

