---
id: TASK-0141
title: Define the project-scoped watcher surface contract
status: done
depends_on: []
priority: normal
tags: [pi-watcher, extension, registration, tdd]
---

# Define the project-scoped watcher surface contract

## Problem

pi-watcher registers six `watcher_*` tools, `/watcher-*` commands, a status bar entry, and lifecycle wiring in every Pi session of every project, even when `.watch.yaml` is absent — spending model context and startup wiring on projects that can never use funzzy.

## Context

PO report: `.tmp/reports/28-08-26/pi-watcher-conditional-registration.md` (problem, outcome, constraints). Lead decisions of 28-08 are binding for this contract:

1. **Gate predicate**: file presence only (`.watch.yaml` or `.watch.yml` in the session project root) — not parsed validity. Missing/invalid `on.socket` and malformed YAML must never decide registration; existing `requireTrustedConfig` errors stay reachable.
2. **Scope**: gate tools, lifecycle effects (polling, subscriptions), and status bar. Slash commands remain always-registered (they do not enter model context; Pi has no unregister API); their handlers keep existing config errors.
3. **Approach**: lazy `registerTools` from `session_start` behind an idempotent guard — no top-level tool registration. On a configured→no-config cwd transition, deactivate watcher tools and reset lifecycle/status where possible.

Known Pi API limit to encode in the contract: dynamically registered tools cannot be unregistered (`getAllTools` only grows). Fresh no-config sessions must register none.

## Acceptance criteria

- [ ] Contract states the gate predicate, the gated vs ungated surfaces, transition semantics per `session_start` re-fire, and the unregister limitation explicitly.
- [ ] Acceptance criteria for TASK-0142/0143 are derived and testable: absent config → no tools/status/polling; present config → behavior identical to today (public compatibility surfaces preserved); both filenames recognized; repeated starts idempotent.
- [ ] Constraints recorded: gate predicate in `domain/` (pure), fs existence check in `infra/`, no Pi imports outside `src/`, gate must not crash extension load on malformed YAML.
- [ ] Non-goals explicit: no mid-session reaction to `.watch.yaml` appearing, no renaming tools/commands, no avoiding entry-module load.

## Verification focus

Document contract only — no code. Bounded handoff for TASK-0142.
