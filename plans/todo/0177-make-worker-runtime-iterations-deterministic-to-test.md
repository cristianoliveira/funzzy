---
id: TASK-0177
title: Make worker runtime iterations deterministic to test
status: todo
depends_on: [TASK-0176]
priority: high
tags: [rust, worker, determinism, tests]
---

# Make worker runtime iterations deterministic to test

## Problem
Even after mechanical extraction, the threaded runtime loop cannot be driven one iteration at a time to prove precedence between scheduler commands and child facts.

## Context

Depends on TASK-0176's private runtime owner. This extends TASK-0171's pure policy tests to runtime orchestration; it must not replace existing domain resolvers or process adapter tests.

## Scope and approach

- Extract one private iteration/command-dispatch boundary from `WorkerRuntime::run`.
- Make the boundary consume explicit accepted scheduler commands and process observations in the existing order. Keep waiting/thread mechanics outside the deterministic step where practical.
- Drive the existing executor using its fake process runner and clock; avoid starting a real worker thread to prove ordering.
- Keep generation and service ownership centralized. Do not copy the production algorithm into test helpers.

## Acceptance criteria

- [ ] Tests drive runtime progress one iteration at a time without wall-clock sleeps or spawned worker threads.
- [ ] Cover successful progress, child failure, accepted cancellation/shutdown competing with child facts, and ignored stale facts.
- [ ] Tests prove no replacement spawn occurs before required reaping/authorization and no stale generation is cancelled.
- [ ] Production `run` delegates to the tested boundary rather than maintaining a second transition implementation.
- [ ] Existing characterization and real-process adapter tests remain unchanged in observable expectations.
- [ ] Document any remaining I/O-bound runtime behavior and its integration coverage; do not claim the full runtime is pure.

## Verification

Use TDD for the iteration seam and inspect branch coverage of the new cases where tooling supports it. Run focused runtime/worker tests, service/cancel/recovery integration coverage, and the fresh watcher final gate. Preserve domain import guards.

## Likely files

`src/workers/runtime.rs`, its colocated tests, and minimal `src/workers.rs` wiring.

## Non-goals

No scheduler algorithm change, concurrency policy change, new public trait hierarchy, or global fake-time configuration.
