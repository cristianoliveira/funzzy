---
id: TASK-0149
title: Trigger the fzz pipeline with one keystroke once it is idle
status: todo
depends_on: []
priority: high
tags: [pi-watcher, ux, shortcut, feedback-loop]
---

# Trigger the fzz pipeline with one keystroke once it is idle

## Problem

Re-running the fzz pipeline today requires typing a slash command or asking the agent to call `watcher_verify`. The user cannot fire the final gate with one keystroke after the watcher finished its current run and is idle, so the feedback loop costs more interaction than it should.

## Outcome

As a user, when the fzz watcher has finished its current run and is idle, I press one keyboard shortcut and the default fzz pipeline (the `@agent-final` gate) starts, with visible confirmation that it started. I never have to type a command or prompt the agent for this.

## Assumptions (bounded, challenge via lead if wrong)

- "Shortcut" = a pi TUI keyboard shortcut registered by `pi-watcher` (`pi.registerShortcut` — capability confirmed in pi 0.84.3 extension docs).
- "The fzz pipeline" = the same default target `watcher_verify` uses (`@agent-final`), keeping one obvious way to define "final gate". Target selection/pickers are out of scope.
- "Once it's finished and is idle" = trigger is effective only when the watcher is idle. If busy at keypress, the press waits for the current generation to finish and then triggers once — no queuing beyond a single press, no auto-re-run loops.
- Respects the existing `.watch.yaml` gate: shortcut is inert (with clear feedback) when the watcher surface is not active.

## Acceptance criteria

- [ ] Pressing the shortcut while the watcher is idle triggers the default final-gate target through the existing control path (`run`/`watcher_verify` semantics — no new Rust protocol method).
- [ ] Pressing the shortcut while a generation is running does not corrupt or cancel it: the press waits for that generation to reach a terminal state, then triggers exactly one new generation.
- [ ] Every effective keypress produces immediate visible feedback (e.g. notification/footer) that the trigger was accepted, and when it is deferred-because-busy.
- [ ] When the watcher surface is inactive (no `.watch.yaml`/`on.socket`, disconnected socket), the shortcut gives one clear message and does nothing else.
- [ ] Repeated/accidental double-presses while a triggered run is pending or running are idempotent — they do not start a second run.
- [ ] Happy and unhappy paths covered by tests following pi-watcher TDD routes (`commands.test.ts`/composition + e2e as applicable); `make quick` and `make all` pass with evidence in `.tmp/reports/`.
- [ ] The shortcut is documented (pi-watcher README/docs) including its default key and how to change it.

## Non-goals

- No new triggering/cancel semantics in the funzzy Rust control protocol.
- No target picker or per-target shortcut variants.
- No scheduled/auto-re-run loops or "run until green" behavior.
- No changes to existing `watcher_*` tool or `/watcher-*` command contracts.

## Notes

- **Lead decision (Mony, 28-08):** sequenced after TASK-0148 (watcher observation UX). Assumptions accepted provisionally; lead validates against pi-watcher patterns/docs before assigning implementation. Sequencing is a priority choice, not a hard dependency — `depends_on` stays empty.

- Feasibility evidence: pi extensions support `pi.registerShortcut(shortcut, { description, handler(ctx) })`; pi-watcher already owns the trigger path via the control socket (`run`) and freshness proof (`watcher_verify`).
- Related but independent: TASK-0148 (exclusive observation feedback) — no dependency.
