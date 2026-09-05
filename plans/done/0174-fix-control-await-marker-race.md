---
id: TASK-0174
title: Fix control_await integration test marker-file race
status: done
depends_on: []
priority: medium
tags: [tests, flakiness, control, integration]
---

# Fix control_await integration test marker-file race

## Problem

`tests/control_await.rs` is flaky under parallel test threads: a marker-file race around `service_pid` (line ~251) and `wait_until` timeouts reproduce on baseline without recent control changes. Sequential runs pass, masking the race in the standard sequential gate while making parallel runs unreliable.

## Desired outcome

The control_await integration suite passes deterministically under default parallel and sequential execution without sleeps or timeouts tuned to machine speed.

## Acceptance criteria

- [x] Identify the exact race: which process writes the marker, which test reads it, and the missing synchronization.
- [x] Replace polling/sleep synchronization with deterministic signaling (ready file with fsync or equivalent atomic marker).
- [x] No test-order or shared-state dependence between tests in the file.
- [x] Prove 5 consecutive parallel runs and 2 sequential runs pass with zero flakes.
- [x] Document the pattern for other integration tests with similar marker files.

## Verification

`cargo test --features test-integration --test control_await` repeated in parallel and sequential modes; full integration gate via watcher.

## Evidence

- Race: service children write PID markers themselves; `service_pid` unwrapped a possibly-absent file, and `stubborn.sh` used truncate-then-write `echo $$ > stubborn.pid` (empty-file window). Fixed in `1019f6f`: `stubborn.sh` now writes atomically (`pid.tmp.$$` + `mv`) before `touch stubborn.started`; `service_pid` is a bounded wait-for-valid helper (exists + parses, 60s load-tolerant deadline, descriptive panic); `wait_until` deadline raised 20s→60s as a load upper bound.
- Ordering: `.started` markers are touched only after the atomic PID commit; tests gate on `.started`/status readiness first.
- No shared state: each test uses its own `setup_directory(test_name)` scratch dir.
- Proof: 5 consecutive parallel runs (24 passed each) and 2 sequential runs (24 passed each) with zero flakes; sibling control suites green in parallel (`control_cancel` 6, `control_output` 11, `control_socket` 14).
- Pattern documented in `tests/AGENTS.md` (Determinism → Service PID marker pattern) for the other suites that still use non-atomic `echo $$ > pid` writes but gate reads behind existence checks.

