---
id: TASK-0159
title: Prove and document default finite-job timeouts
status: done
depends_on: [TASK-0158]
priority: high
tags: [integration-tests, docs, timeout, jobs, reliability]
---

# Prove and document default finite-job timeouts

## Problem
Users need black-box evidence and discoverable documentation showing when the default applies, when a per-job timeout wins, and how timeout failures appear across local and watched pipelines.

## Acceptance criteria

- [x] Add black-box coverage showing a finite job without `jobs[].timeout` inherits the configured execution default, terminates its process tree, becomes `timedout`, and fails the run/generation.
- [x] Prove a job-specific timeout deterministically overrides the default, including a shorter custom budget and a longer custom budget.
- [x] Prove configurations with neither timeout remain unbounded and retain existing success/failure behavior.
- [x] Prove a managed service remains unbounded under the execution default while a direct job timeout plus `service: true` remains an actionable configuration error.
- [x] Prove local `fzz run`, watched/control execution, human output, structured snapshot/event state, retained evidence, failure hooks, and exit status agree with existing timeout semantics.
- [x] Prove changing only the default through hot reload affects a later generation and cannot expire an already-scheduled generation.
- [x] Prove `fzz check`, configuration schema/section output, generated examples/init template, and rendered config describe the default and override precedence consistently.
- [x] Document one concise default-plus-override example in README, USAGE, and advanced guidance, and retain the distinction from control-client `--timeout`.
- [x] Update the finite-job timeout contract or add a focused amendment so per-job and default semantics have one normative source.
- [x] Run focused Rust tests, integration checks, config discovery/init proof, and the configured watcher/fallback final gate.

## Test constraints

Use injected clocks or synchronization for correctness. Outer harness deadlines may guard hangs, but assertions must not depend on narrow wall-clock thresholds.

## Verification evidence

- `cargo fmt --all -- --check`: passed.
- `cargo test`: passed across 58 test binaries.
- `cargo test --features test-integration --test '*' -- --nocapture --test-threads=1`: passed across 54 integration binaries.
- `finite_job_timeouts`: 9/9 passed, including inherited default, override, services, local/control, reload, cancellation, evidence, and process-tree cleanup.
- Watcher generation 12 passed format and unit tests on the inherited-default change. Final integration verification later disconnected, so the exact local integration command above was used as the deterministic fallback.

## Non-goals

- Re-proving every per-job timeout race already covered by TASK-0140.
- Introducing a new timeout outcome, protocol method, or agent-facing wait behavior.
