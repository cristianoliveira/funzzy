---
id: TASK-0176
title: Extract worker consumer runtime from construction
status: done
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

- [x] Worker construction reads as dependency wiring plus thread startup, without embedded lifecycle transitions.
- [x] Runtime state has one explicit owner; extraction adds no new locks, detached threads, or hidden globals.
- [x] Existing public constructor paths and defaults remain compatible.
- [x] Generation allocation, scheduler command precedence, cancellation, hook timing, and service handoff behavior remain unchanged.
- [x] TASK-0175 characterization tests pass without weakening their assertions.
- [x] Review the move diff for ordering changes and document the new ownership boundary.

## Verification

Run focused worker tests and feature-enabled executor-contract tests, then the fresh watcher final gate. Include cancellation, recovery, service, reload, and shutdown coverage. Compare module dependencies before/after; do not interpret moved lines as reduced behavioral complexity.

## Evidence

- Construction boundary: `src/workers/mod.rs` now wires `Scheduler`, executor adapters, hooks, and `WorkerRuntime`, then starts one named runtime thread; it contains no lifecycle loop. `Worker` remains the public submission/lifetime handle.
- Single owner: private `WorkerRuntime` in `src/workers/runtime.rs` owns `scheduler`, `events`, `executor`, `hook_context`, and all loop-local active/pending/service/settlement state. No new locks, detached threads, globals, or public exports.
- Compatibility: existing constructor signatures/default approval paths compile and pass the worker, control, reload, service, and shutdown suites unchanged.
- Behavior preservation: `cargo test --lib workers::` passed 56; `cargo test --test domain_boundaries` passed 8; feature-enabled `cargo test --features test-integration --test executor_contract` passed 2. These include cancellation, recovery, service handoff, reload, and shutdown cases, plus TASK-0175 ordering characterization tests.
- Move review: `git diff --find-renames=50% --check HEAD~1..HEAD -- src/workers.rs src/workers/mod.rs src/workers/runtime.rs` is clean. Extracted body is statement-order/state-representation identical; only `consumer_scheduler`→`scheduler`, `service_events`→`events`, indentation, and rustfmt line reflow differ. Ownership boundary is documented in `runtime.rs`.
- Module dependency comparison: `ast_module_graph` before (archived `HEAD~1`, 61 files) and after (current, 62 files) both report 38 modules, identical `workers` fan-out to `duration_recorder,duration_store,executor,output,path_context,plan,rules,stdout,template`, no new worker edge, and the same pre-existing cycles (`cli↔config`, `executor↔stdout`). `runtime.rs` uses only `super::*`; it adds no crate-level dependency edge.
- Domain guard decision: `workers::runtime` is private runtime orchestration, not a domain-facing module. It coordinates executor/process, scheduler, service pool, stdout, and hook settlement; it does not import or expose `crate::domain`. `tests/domain_boundaries` remains unchanged and passes, so no guard extension is warranted.
- Fresh gate: watcher generation 164 passed on the unchanged post-extraction fingerprint.


## Likely files

`src/workers.rs`, private `src/workers/runtime.rs`, and associated colocated tests.

## Non-goals

No generic runtime framework, constructor renaming, options-builder migration, new public exports, or changed wire events.
