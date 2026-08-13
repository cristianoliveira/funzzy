---
id: TASK-0007
title: Decompose pi-watcher registration composition
status: todo
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

- [ ] `index.ts` continues to own concrete dependency wiring.
- [ ] Tool registration, command registration, and polling lifecycle have focused modules or factories.
- [ ] Domain and application layers remain independent from Pi APIs.
- [ ] Trust and config lookup policy has one reusable boundary.
- [ ] Session start/shutdown remains idempotent.
- [ ] Existing tool names, command names, and responder behavior remain unchanged.

## Verification

- Registration tests assert behavior through public Pi extension surface.
- Lifecycle, responder, stable-run, and failure-notifier tests pass.
- Pi watcher `make all` passes.

