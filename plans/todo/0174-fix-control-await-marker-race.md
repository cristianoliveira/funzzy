---
id: TASK-0174
title: Fix control_await integration test marker-file race
status: todo
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

- [ ] Identify the exact race: which process writes the marker, which test reads it, and the missing synchronization.
- [ ] Replace polling/sleep synchronization with deterministic signaling (ready file with fsync or equivalent atomic marker).
- [ ] No test-order or shared-state dependence between tests in the file.
- [ ] Prove 5 consecutive parallel runs and 2 sequential runs pass with zero flakes.
- [ ] Document the pattern for other integration tests with similar marker files.

## Verification

`cargo test --features test-integration --test control_await` repeated in parallel and sequential modes; full integration gate via watcher.
