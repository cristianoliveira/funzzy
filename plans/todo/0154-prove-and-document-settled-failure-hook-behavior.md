---
id: TASK-0154
title: Prove and document settled failure hook behavior
status: todo
depends_on: [TASK-0153]
priority: high
tags: [integration-tests, docs, hooks, config, watcher, reliability]
---

# Prove and document settled failure hook behavior

## Problem
Delayed failure-hook behavior crosses scheduling, cancellation, finite runs, reloads, and compatibility; without black-box proof and documentation agents may act on stale failures.

## Outcome

Prove the accepted contract at the CLI and watcher boundaries and teach users when to choose immediate versus settled custom failure hooks.

## Acceptance criteria

- [ ] Black-box test proves a stable failed generation runs the custom command exactly once after the settle boundary.
- [ ] Black-box test proves a newer generation can start before the old settle duration expires.
- [ ] Tests prove newer pass suppression, repeated-failure coalescing, cancellation/supersession, watcher shutdown, and custom-command failure.
- [ ] Tests cover the contracted finite-run, control-await, valid reload, and malformed-reload behavior.
- [ ] Existing immediate scalar success/failure hook tests remain green and demonstrate compatibility.
- [ ] Schema, generated config example, `fzz check`, README/USAGE, and `docs/RUN-HOOKS-CONTRACT.md` agree on syntax and lifecycle.
- [ ] Documentation says settlement is based on watcher generations, not knowledge of agent activity.
- [ ] Documentation warns that external side effects cannot be recalled once command execution begins.
- [ ] Verification includes focused tests plus configured final watcher gates; failure evidence is retained in the task notes.

## Constraints

- Use bounded polling or lifecycle events in integration tests, never timing-only fixed sleeps.
- Keep examples generic: the command may call any user-owned script.

## Notes

QA should challenge boundary races rather than treating elapsed wall time alone as proof.

