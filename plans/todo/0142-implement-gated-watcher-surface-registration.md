---
id: TASK-0142
title: Implement gated watcher surface registration
status: todo
depends_on: [TASK-0141]
priority: normal
tags: [pi-watcher, extension, registration, tdd]
---

# Implement gated watcher surface registration

## Problem

The composition root must skip watcher tools and lifecycle when the session project lacks the watcher contract, without weakening any behavior for configured projects.

## Context

Implement only the approved TASK-0141 contract. Composition root is `pi-watcher/src/index.ts`; registration lives in `src/tools.ts` / `src/commands.ts`; lifecycle in `src/polling.ts`. Registration tests belong in `src/commands.test.ts` (see `pi-watcher/AGENTS.md` task routes).

- Add a pure domain predicate for watcher-contract presence and an `infra/` existence adapter (`.watch.yaml`/`.watch.yml`, session `ctx.cwd`).
- Move `registerTools` behind an idempotent `session_start` guard; commands stay top-level.
- Gate lifecycle effects and status bar on the same predicate; on configured→no-config cwd transitions deactivate/reset where Pi allows.

## Acceptance criteria

- [ ] Failing tests first: absent config → no watcher tools registered, no polling started; present config → tools/commands/status identical to today.
- [ ] Both `.watch.yaml` and `.watch.yml` recognized; `.yml` loses to `.yaml` deterministically (match `readConfig` order).
- [ ] Malformed YAML or invalid/missing `on.socket` still registers the surface — errors surface at tool-call time via `requireTrustedConfig` as today.
- [ ] Repeated `session_start` fires and cwd switches are idempotent; no duplicate tool registrations, no orphaned polling.
- [ ] No background work (sockets, timers, subscriptions) started in no-config sessions.
- [ ] Dependencies point inward: predicate in `domain/`, fs in `infra/`, wiring only in `src/index.ts`.

## Verification focus

Keep registration/lifecycle tests beside `src/commands.test.ts`. End-to-end loop behavior stays in TASK-0143.
