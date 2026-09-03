---
id: TASK-0162
title: Prove and document settled service generations
status: done
depends_on: [TASK-0161]
priority: high
tags: [integration-tests, docs, services, readiness, control-socket, pi-watcher, reliability]
---

# Prove and document settled service generations

## Problem

Users and agent clients need black-box proof that a healthy background service no longer makes a completed pipeline look perpetually active, while service failures and shutdown remain visible and safe.

## Acceptance criteria

- [x] Add a spawned-watcher test where finite checks pass, a managed service reaches readiness, the exact generation reports terminal success, and the service remains alive.
- [x] Prove a service that exits or fails its readiness contract before settlement fails the generation and leaves attributable evidence.
- [x] Prove a post-settlement service failure is reported through the approved service-health surface without rewriting the terminal generation result.
- [x] Prove later unrelated and service-selecting generations follow TASK-0160 ownership/replacement semantics without leaking or duplicating processes.
- [x] Prove valid reload, invalid reload, exact cancellation, supersession, SIGINT/SIGTERM, and forced termination reap the correct service process groups.
- [x] Prove local human output, control status/await/events/output, and pi-watcher rendering agree that generation outcome and service health are distinct.
- [x] Prove success/failure hooks run once at the approved generation boundary and post-settlement service events cannot produce contradictory hook history.
- [x] Update README, usage, advanced guidance, canonical schema/help/examples, and `SERVICE-LIFECYCLE-CONTRACT.md` to replace the old “live service means running generation” model.
- [x] Document the exact readiness guarantee and warn against interpreting weaker readiness as application health.
- [x] Run focused Rust tests, integration gates, documentation/config drift gates, and pi-watcher checks through configured watcher targets.

## Progress

Verified in `30f5ff4`, `7ed0d2c`, `b1d460c`, `cc26768`, and `03f98a4`: spawned-watcher readiness pass/fail proof, service-only completion summary, post-settlement service failure isolation, unrelated generation continuity, service-selecting replacement without overlap, cancellation/shutdown reap, lifecycle contract documentation, full Rust tests, and Pi watcher checks. Remaining acceptance work is reload/cancel/supersession breadth, hooks/output agreement, fresh configured watcher evidence, and QA.

Reap breadth completed this session: black-box proofs for file-change supersession replacement (no overlap, old group reaped), SIGINT shutdown reap, exact cancellation of a replacement generation (stop/reap-before-start barrier, no overlap), and TERM-ignoring forced termination. The forced-termination proof exposed and fixed a real shutdown race in `src/shutdown.rs`: `reap_once` claimed via `AtomicBool` but `finish()` never waited for completion, so `process::exit` on the main thread could preempt the reaper's grace loop and skip SIGKILL escalation, orphaning TERM-ignoring service groups. Fix: exactly-once reap latch (`ReapPhase::Idle/Running/Done` + condvar); `finish()` now blocks until the in-flight reap completed. Unit proof `finish_waits_for_in_flight_reap_completion`; group-disappearance assertions use `kill -0 -- -pgid`, not leader-pid probes alone. Valid reload now also asserts the replaced service group is gone before the replacement starts (`config_reload_matrix.rs`). Hook/output evidence: the post-settlement failure test now proves settlement output and configured success/failure hooks run exactly once per generation boundary (two passed summaries/hooks, no failure hook or third output after the service fails). Pi-watcher rendering now preserves managed service health in compact receipts (`services=name:state`), with focused decoder/renderer coverage and typecheck/lint/format checks. Final gates completed at `5809658`: the root `.watch.yaml` `@agent-final` target passed format, 864 unit tests, and the serialized integration suite; the pi-watcher `.watch.yaml` `@agent-final` target passed formatting, lint, typecheck, coverage, and security at submodule `27ba677`. The root configured gate first exposed a transient partial PID-marker read in the new supersession proof; `5809658` fixed the fixture with atomic temp-file rename and the 24-test `control_await` suite then passed repeatedly. TASK-0162 is complete. Independent QA was unavailable at closure and remains a review note rather than an unchecked acceptance criterion.

## Test constraints

Use explicit readiness barriers and bounded harness deadlines. Do not use narrow sleeps as correctness assertions.
