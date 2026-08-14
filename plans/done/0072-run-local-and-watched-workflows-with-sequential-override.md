---
id: TASK-0072
title: Run local and watched workflows with sequential override
status: done
depends_on: [TASK-0071, TASK-0027]
priority: high
tags: [rust, cli, executor, concurrency, debugging, tdd]
---

# Run local and watched workflows with sequential override

## Problem
The contract needs an explicit local/session execution path that caps task concurrency at one without rewriting configuration while preserving all other plan and process semantics.

## Context

Wire execution policy from Clap composition root into shared executor; do not branch or duplicate scheduling engine.

## Acceptance criteria

- [ ] Parser tests first cover `fzz run TARGET --sequential`, `fzz watch TARGET --sequential`, both aliases, global-option placement policy, help, and unknown/conflicting input.
- [ ] Deterministic fake-process tests prove maximum active tasks is one while preserving original parallel group occurrences/barriers.
- [ ] Commands inside each task remain sequential and all selected sibling tasks still run unless fail-fast stops them.
- [ ] Local run and watch wait/restart modes use shared executor with effective concurrency one and no config rewrite/reload.
- [ ] Configured concurrency remains observable separately from effective override and default invocation behavior is unchanged.
- [ ] Execution signature/duration estimate uses effective one; returning to parallel selects original profile.
- [ ] `fzz` and `funzzy` output/exit/help parity is black-box tested.
- [ ] Ctrl-C/restart reaps same process groups and no extra scheduler threads remain.
- [ ] Verbose diagnostic states override once without noisy per-command repetition.

## Notes

