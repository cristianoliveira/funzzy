---
id: TASK-0150
title: Trigger the fzz pipeline with one keystroke once it is idle
status: done
depends_on: []
priority: high
tags: [rust, cli, watch-loop, ux, stdin, tdd]
---

# Trigger the fzz pipeline with one keystroke once it is idle

## Problem

While `fzz` is watching, re-triggering the pipeline requires saving/touching a watched file or opening a second terminal for `fzz control run <target> --wait`. The human sitting in the watcher terminal cannot simply press a key to run the pipeline again once the current run finished and the loop is idle. The feedback loop costs one extra terminal or one fake file edit.

## Outcome

As a user running `fzz` watch, when the current generation finished and the loop is idle, I press one key in the watcher terminal and the full pipeline starts — same scheduling, correlation, and output I get from any other trigger. If a run is in flight, my press waits for it to finish and then triggers exactly once.

## Assumptions (bounded, challenge via lead if wrong)

- Scope is the **fzz loop itself** (Rust CLI), NOT pi-watcher — confirmed by requester.
- "The fzz pipeline" = the full configured workflow (every job eligible to run), routed through the same scheduler path as control-socket runs (`control run` semantics, generation-correlated, synthetic-trigger path) — not a bespoke bypass.
- "Shortcut" = a single keystroke read from the watcher terminal's stdin. Exact key is dev's choice: must not collide with existing terminal behavior (Ctrl-C/SIGINT stays intact), must be documented.
- "Once it's finished and is idle" = effective only when no generation is running; a press during a run waits for terminal, then triggers once. No queue beyond a single outstanding press, no auto-re-run loops.
- Applies to the watch modes that own the terminal stdin; if a mode structurally cannot (documented reason), that exclusion is stated in docs and tests.

## Acceptance criteria

- [ ] Pressing the key while idle triggers one new generation covering the full pipeline through the existing run path — observable via control `status`/`output` exactly like any other generation (no parallel ad-hoc execution path).
- [ ] Pressing the key while a generation is running does not cancel or corrupt it; the press is latched and fires exactly one new generation after the current one reaches a terminal state.
- [ ] Additional presses while a trigger is pending or running are idempotent — never two queued runs.
- [ ] Existing terminal behavior preserved: Ctrl-C/SIGINT shutdown unchanged; normal task output remains readable (no raw-mode garbage); behavior without a TTY (piped/closed stdin) is safe and documented.
- [ ] Key decode → trigger policy is pure domain logic with unit tests (happy + unhappy: unknown keys, partial sequences, EOF); integration test drives the loop via piped stdin — deterministic, no real-TTY dependency, no sleeps beyond bounded waits.
- [ ] Both watch modes (blocking and non-blocking/restart) either support the shortcut or carry an explicit tested+documented exclusion.
- [ ] The key (and how to change it, if configurable) is documented in README/docs and the commented init template stays consistent with it.
- [ ] `make lint`, `make tests`, `make integration` pass; evidence recorded in `.tmp/reports/`.

## Non-goals

- No interactive TUI, menus, or target selection — one key, full pipeline.
- No changes to pi-watcher.
- No new control-socket protocol methods (reuse the internal run path).
- No scheduled/auto-re-run or "run until green" behavior.

## Notes

- Supersedes TASK-0149: that intake was initially shaped as a pi TUI keybinding and implemented in pi-watcher (historical, kept, not extended). Requester clarified the intent was always the fzz watch loop itself; this task carries the corrected scope. QA: independent verification assigned to Kely (lead decision).
- Feasibility: `src/watch_loop.rs` currently has no stdin handling — this adds a stdin reader alongside the filesystem event stream; trigger routing already exists (control-socket `run`, synthetic path events).
