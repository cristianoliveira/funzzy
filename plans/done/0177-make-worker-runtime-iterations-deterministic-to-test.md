---
id: TASK-0177
title: Make worker runtime iterations deterministic to test
status: done
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

- [x] Tests drive runtime progress one iteration at a time without wall-clock sleeps or spawned worker threads.
- [x] Cover successful progress, child failure, accepted cancellation/shutdown competing with child facts, and ignored stale facts.
- [x] Tests prove no replacement spawn occurs before required reaping/authorization and no stale generation is cancelled.
- [x] Production `run` delegates to the tested boundary rather than maintaining a second transition implementation.
- [x] Existing characterization and real-process adapter tests remain unchanged in observable expectations.
- [x] Document any remaining I/O-bound runtime behavior and its integration coverage; do not claim the full runtime is pure.

## Verification

Use TDD for the iteration seam and inspect branch coverage of the new cases where tooling supports it. Run focused runtime/worker tests, service/cancel/recovery integration coverage, and the fresh watcher final gate. Preserve domain import guards.

## Closure evidence

- Existing expectations unchanged: `4d76d32` changes only `src/workers/runtime.rs`; no existing characterization or real-process adapter test was edited. The unchanged worker characterization suite passes (62 tests), including TASK-0175 event ordering. The adapter-facing integration suites below pass with their existing observable assertions.
- Focused service/reload coverage: `cargo test --features test-integration --test control_await` — 24 passed (readiness lifecycle, service settlement/failure, cancellation, shutdown, replacement, and reaping); `cargo test --features test-integration --test config_reload_matrix` — 7 passed (including service signature replacement without process exit and reload preservation).
- Focused cancellation coverage: `cargo test --features test-integration --test control_cancel` — 6 passed (active cancellation, exact await, terminal/unknown/stale no-op, descendant cleanup).
- Focused recovery coverage: `cargo test --features test-integration --test recovery_pty` — 9 passed (approval, verification, failure, timeout, cancellation/supersession, ordered multi-command recovery).
- Remaining I/O-bound behavior: `WorkerRuntime::run` still owns the real worker thread, scheduler condvar/deadline waiting, executor `ProcessRunner`/`ChildProcess` polling and shutdown, `SystemClock`, recovery approval, stdout presentation, service-pool polling/handoff, and settled-hook process lifecycle. These remain intentionally outside `RuntimeIteration::apply`; the seam tests orchestration decisions only and do not claim the full runtime is pure.
- Integration mapping: scheduler/thread behavior is covered by the worker and control suites; process-group/reaping and readiness/service behavior by `control_await` and `config_reload_matrix`; recovery process/PTY behavior by `recovery_pty`; domain import direction remains covered by `cargo test --test domain_boundaries` (8 passed). Fresh watcher generation 170 (`integration @agent-final`) passed on fingerprint `0ce1475c273c`. No runtime policy or second implementation was added for closure documentation.


## Likely files

`src/workers/runtime.rs`, its colocated tests, and minimal `src/workers.rs` wiring.

## Non-goals

No scheduler algorithm change, concurrency policy change, new public trait hierarchy, or global fake-time configuration.
