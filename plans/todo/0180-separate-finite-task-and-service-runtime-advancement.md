---
id: TASK-0180
title: Separate finite task and service runtime advancement
status: doing
depends_on: [TASK-0179]
priority: high
tags: [rust, executor, services, cohesion]
---

# Separate finite task and service runtime advancement

## Problem
Executor advance_task combines finite deadlines and outcomes with service restart handling, process adaptation, capture, and presentation, making lifecycle changes hard to review.

## Context

At review baseline, `src/executor.rs::advance_task` spans about 297 lines, handling working-directory validation, capture, spawn/poll, deadlines, service restart/backoff, and failures. TASK-0171 already extracted pure finite/service decisions. Extend those seams instead of creating another state machine. TASK-0179 establishes the result-contract owner before reorganizing executor internals.

## Scope and approach

- Map current finite-job versus service advancement and shared process effects. Characterize any missing cases before extraction.
- Split private runtime methods/modules by finite execution and service lifecycle responsibilities, leaving shared process mechanics explicitly shared.
- Keep cancellation/deadline precedence in existing domain resolvers; keep runtime process types out of domain modules.
- Isolate presentation through a narrow injected diagnostic sink or existing suitable internal contract only where direct output prevents testing. Do not invent public wire events.
- Coordinate touched executor methods with worker-runtime work; unrelated task dependencies are not required merely because both use the executor.

## Acceptance criteria

- [ ] Top-level advancement exposes finite/service intent without inlining both lifecycle implementations.
- [ ] There is one owner for each policy; no duplicate finite lifecycle resolver or replacement scheduler is introduced.
- [ ] Fake runner/clock tests cover sequential continuation, successful completion, spawn/poll failure, timeout, cancellation, service restart exhaustion, and readiness handoff.
- [ ] Job-wide deadlines do not reset on continuation; cancellation/timeout precedence remains unchanged.
- [ ] Output capture, live messages, result attribution, recovery, fail-fast, and service reaping/replacement semantics remain unchanged.
- [ ] Public executor paths, wire types, and event ordering remain compatible.

## Verification

Run focused executor/domain/service tests and feature-enabled timeout, process-group, recovery, cancellation, service, and output suites. Use the fresh watcher final gate. Compare responsibilities and dependencies before/after; Rust AST complexity scores that omit branches are not acceptance evidence.

## First bounded seam evidence

- Method ownership and criterion matrix: `.tmp/reports/04-09-26/task-0180-finite-dispatch.md`.
- Mechanical extraction commit `08d40eb`: `advance_task` retains context validation and readiness-start dispatch; the former inline 261-line execution loop is now `advance_finite_task`. Exact old/new loop comparison is byte-identical. `advance_starting_service`, `advance_services`, policy resolvers, adapters, public APIs, events, and state representation are unchanged.
- Focused characterization after extraction: executor fake-runner/clock tests 67 passed; serial `finite_job_timeouts` 9 passed; serial `control_await` 24 passed. Fresh watcher generation 191 passed (`cargo fmt --all -- --check && cargo test`).
- Remaining criteria: service-specific advancement still shares the moved loop's restart branch; output/recovery/fail-fast/service effects remain shared and must be preserved in the next bounded seam. No policy or process abstraction changes were introduced.

## Discovery evidence

- Criterion matrix and responsibility inventory: `.tmp/reports/04-09-26/task-0180-discovery.md`.
- Pre-move characterization: executor fake-runner/clock suite 67 passed; serial feature `finite_job_timeouts` 9 passed; serial feature `control_await` 24 passed. These constrain sequential continuation, spawn/poll failure, timeout/cancellation precedence, service restart/readiness/handoff, reaping, and recovery-adjacent worker behavior.
- Existing pure policy owners are preserved: `src/domain/finite_lifecycle.rs` and `src/service_lifecycle.rs`; no missing characterization test was identified and no production code was changed in this discovery slice.

## Likely files

`src/executor.rs`, private executor submodules if justified, existing output/diagnostic adapter wiring, and colocated tests.

## Non-goals

No second execution engine, new crates, timeout policy changes, service algorithm changes, or relocation of runtime-specific process traits into the domain.
