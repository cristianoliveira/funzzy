---
id: TASK-0090
title: Swap valid watch roots and execution policy without process exit
status: todo
depends_on: [TASK-0089, TASK-0086, TASK-0035, TASK-0040, TASK-0041]
priority: high
tags: [rust, watcher, config, reload, executor, services, tdd]
---

# Swap valid watch roots and execution policy without process exit

## Problem
A valid candidate must update matching roots jobs concurrency debounce ignore hooks and services atomically while preserving PID and avoiding event-loss gaps or duplicated generations.

## Context

Use prepare→commit→retire transaction. Added resources become ready before pointer swap; obsolete resources retire only after commit.

## Acceptance criteria

- [ ] Tests first cover root add/remove/overlap, job add/remove/rename, matching/ignore, concurrency, debounce, backend, hooks/output policy, managed service signature, and socket path changes.
- [ ] Candidate registers all added native/poll roots and starts required backend/control resources before commit; any failure invokes invalid fatal shutdown without partial live mutation.
- [ ] Commit atomically swaps runtime config and routes later batches to new revision; obsolete roots/backend resources retire after boundary without event-loss gap.
- [ ] Duplicate events observed by overlapping old/new roots are normalized once with revision/batch identity.
- [ ] Active finite tasks keep old revision and complete unless existing busy/cancellation policy explicitly applies; valid config save alone does not kill them.
- [ ] Managed services unchanged by execution signature remain owned; changed/removed services receive graceful replacement/removal with bounded kill/reap and new services start under new revision.
- [ ] Concurrency/policy changes affect only newly planned generation and never resize currently running group inconsistently.
- [ ] Control socket path change uses bind-new-before-retire-old handoff or equivalent safe strategy while process remains alive; bind failure takes fatal path.
- [ ] Config watcher remains anchored to parent so atomic replace/delete/recreate is observed after root swap.
- [ ] Logging truncate-on-change occurs only for committed valid semantic reload and preserves deterministic notice order.

## Notes
