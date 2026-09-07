---
id: TASK-0176
title: Extract worker consumer runtime from construction
status: todo
depends_on: [TASK-0175]
priority: high
tags: [rust, architecture, worker, cohesion]
---

# Extract worker consumer runtime from construction

## Problem
Worker construction embeds the long-lived runtime loop and concrete adapter wiring, so readers and tests must handle both responsibilities together.

## Context

At review baseline, `src/workers.rs:944-1469` constructs adapters and embeds a roughly 470-line consumer closure. TASK-0175 supplies the invariant baseline. TASK-0171's existing pure lifecycle policy remains the decision owner.

## Scope and approach

- Introduce a private `WorkerRuntime` in `src/workers/runtime.rs` to own active/pending runs, service coordination, settlement ownership, and the consumer loop.
- Pass the existing configured `Executor`, scheduler, event handles, and hook context into the runtime. Do not create duplicate process/clock abstractions.
- Keep thread spawning and dependency construction at the construction boundary. `Worker` remains the submission and lifetime handle.
- Perform a mechanical move first: preserve statement ordering and current state representation. Keep the iteration redesign in TASK-0177.

## Acceptance criteria

- [ ] Worker construction reads as dependency wiring plus thread startup, without embedded lifecycle transitions.
- [ ] Runtime state has one explicit owner; extraction adds no new locks, detached threads, or hidden globals.
- [ ] Existing public constructor paths and defaults remain compatible.
- [ ] Generation allocation, scheduler command precedence, cancellation, hook timing, and service handoff behavior remain unchanged.
- [ ] TASK-0175 characterization tests pass without weakening their assertions.
- [ ] Review the move diff for ordering changes and document the new ownership boundary.

## Verification

Run focused worker tests and feature-enabled executor-contract tests, then the fresh watcher final gate. Include cancellation, recovery, service, reload, and shutdown coverage. Compare module dependencies before/after; do not interpret moved lines as reduced behavioral complexity.

## Likely files

`src/workers.rs`, private `src/workers/runtime.rs`, and associated colocated tests.

## Non-goals

No generic runtime framework, constructor renaming, options-builder migration, new public exports, or changed wire events.
