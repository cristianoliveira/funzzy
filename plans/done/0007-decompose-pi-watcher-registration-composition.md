---
id: TASK-0007
title: Decompose pi-watcher registration composition
status: done
depends_on: []
priority: low
tags: [pi-watcher, architecture, cohesion]
---

# Decompose pi-watcher registration composition

## Problem

`pi-watcher/src/index.ts` wires dependencies correctly but its main registration function also contains polling lifecycle, failure delivery, three tools, three commands, repeated trust/config checks, and UI behavior.

## Scope

Keep `index.ts` as composition root while extracting focused registration and trusted-config adapters.

## Acceptance criteria

- [x] `index.ts` continues to own concrete dependency wiring.
- [x] Tool registration, command registration, and polling lifecycle have focused modules or factories.
- [x] Domain and application layers remain independent from Pi APIs.
- [x] Trust and config lookup policy has one reusable boundary.
- [x] Session start/shutdown remains idempotent.
- [x] Existing tool names, command names, and responder behavior remain unchanged.

## Verification

- [x] Registration tests assert behavior through public Pi extension surface.
- [x] Lifecycle, responder, stable-run, and failure-notifier tests pass.
- [x] Pi watcher `make all` passes.

## Outcome

`src/index.ts` (316 lines) decomposed into focused modules, all in the Pi-facing layer with `index.ts` as entry + composition root:

- `src/trusted-config.ts` — single trust + config boundary (`createRequireTrustedConfig`, `TrustedConfigError` with `untrusted`/`not-configured` kinds) shared by all tools and commands.
- `src/polling.ts` — `createPollingLifecycle` owns polling state machine, failure notifier, activity attribution; start/shutdown idempotent.
- `src/tools.ts` — `registerTools` registers `watcher_status`, `watcher_targets`, `watcher_verify`.
- `src/commands.ts` — `registerCommands` registers the three `/watcher-*` commands; maps `TrustedConfigError` to warning notifications.
- `src/domain/targets-presentation.ts` — `formatTargets` moved to domain presentation (was inline in index).

Factories receive dependencies injected from `index.ts` (no module-level infra imports). Tests: factory units with injected fakes + composition-root tests through the public Pi surface; 103 tests pass, `make all` green (functions coverage 95.74%).

