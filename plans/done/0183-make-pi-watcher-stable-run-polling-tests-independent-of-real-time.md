---
id: TASK-0183
title: Make Pi watcher stable-run polling tests independent of real time
status: done
depends_on: []
priority: normal
tags: [pi-watcher, typescript, determinism, tests]
---

# Make Pi watcher stable-run polling tests independent of real time

## Problem
waitForRun injects status reads but uses the real clock and timers. Its tests use millisecond sleeps instead of controlling exact deadline and update boundaries.

## Context

`pi-watcher/src/application/stable-run.ts::waitForRun` uses `Date.now` and `setTimeout` directly. Its colocated tests poll at 1 ms within 500 ms real-time budgets. `application/observe.ts` already supports an injected `now` callback; inspect local patterns before choosing a seam.

This is work in the separate `pi-watcher` submodule. Follow its application guide and preserve any existing uncommitted changes. Commit implementation in that repository; update the parent pin only to the intended committed revision through the normal submodule workflow.

## Scope and approach

- Use a small explicit `now`/`sleep` dependency or local fake timers to control deadline and update boundaries. Choose the smallest option consistent with nearby tests.
- Preserve the public `waitForRun` call shape/default behavior, including exact-generation completion and supersession semantics.
- Extend tests for missing terminal/error paths; do not claim mocked status reads verify real socket behavior.

## Acceptance criteria

- [x] Stable-run tests do not depend on wall-clock sleeps or host scheduling budgets.
- [x] Cover exact-generation success and failure terminals, older-generation rejection, newer-generation supersession, read failure propagation, deadline boundary, and periodic updates.
- [x] Timeout and update assertions advance deterministic time rather than waiting in real time.
- [x] Timers/mocks are cleaned after each case; tests remain independent under parallel execution.
- [x] Production timeout/update intervals, messages, return values, and caller compatibility remain unchanged.
- [x] Existing socket/application integration tests continue to cover real transport separately.

## Verification

Run focused stable-run and affected verification tests, then the submodule's configured quick/final gates, including coverage thresholds. Inspect coverage for timeout and supersession paths. The parent Rust watcher pass is not proof of TypeScript verification; use the submodule watcher when available.

## Likely files

`pi-watcher/src/application/stable-run.ts`, `stable-run.test.ts`, and minimal caller wiring only if explicit dependencies require it.

## Evidence and closure

- Submodule characterization and pre-existing-dirt inventory: `.tmp/reports/04-09-26/task-0183-stable-run.md`.
- Submodule implementation commit: `3acadb8`; parent gitlink pin: `92fb34d`.
- `make quick` passed: 40 test files / 531 tests, format, lint, and typecheck.
- `make all` passed: coverage thresholds exceeded (93.53% statements, 87.97% branches, 96.82% functions, 95.08% lines) and npm audit found no high-severity vulnerabilities.
- No Rust watcher verification was used as TypeScript proof. Pre-existing parent `Cargo.lock` dirt remains untouched.

## Non-goals

No general timer service, socket retry redesign, public tool changes, or broad application clock migration.
