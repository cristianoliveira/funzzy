---
id: TASK-0140
title: Prove and document finite-job execution timeouts
status: todo
depends_on: [TASK-0139]
priority: normal
tags: [integration-tests, docs, timeout, control-socket, process, reliability]
---

# Prove and document finite-job execution timeouts

## Problem

Users and agent clients need black-box proof that execution deadlines terminate process trees and remain distinct from client wait deadlines, cancellation, and ordinary command failure.

## Acceptance criteria

- [ ] Add a spawned-binary test where a finite job exceeds configured timeout and reaches the approved terminal timeout outcome.
- [ ] Prove the timed-out child and descendant process tree are gracefully terminated, escalated when necessary, and reaped.
- [ ] Prove output emitted before termination remains bounded, attributable, and retrievable by exact generation.
- [ ] Prove natural success and failure before deadline retain their ordinary outcomes.
- [ ] Prove exact user cancellation before deadline wins according to TASK-0138 and cannot later become timeout.
- [ ] Prove reload changes timeout only for later generations and stale timer activity cannot affect replacement work.
- [ ] Prove local `fzz run`, watcher status/await, structured control output, and pi-watcher decoding agree on timeout semantics.
- [ ] Document clearly that `jobs[].timeout` owns child lifetime while control `--timeout` owns only caller wait duration.
- [ ] Add the optional timeout to integration-agnostic command-observation guidance without making it provider-specific.
- [ ] Update README, USAGE, ADVANCED-GUIDE, canonical schema/help/examples, and compatibility contracts required by the approved surface.
- [ ] Run focused Rust tests, integration gate, docs/config drift gates, and pi-watcher checks through configured watcher targets.

## Test constraints

Use synchronization to establish process start and timeout observation. Outer harness deadlines may be generous safety bounds, but assertions must not depend on narrow wall-clock timing.
