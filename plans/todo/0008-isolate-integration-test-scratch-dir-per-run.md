---
id: TASK-0008
title: Isolate integration test scratch dir per run
status: todo
depends_on: [TASK-0002]
priority: high
tags: [tests, concurrency, reliability]
---

# Isolate integration test scratch dir per run

## Problem

Integration runs share one global scratch dir `/tmp/fzz` (wiped with `rm -rf`
by `.watch.yaml` and `Makefile` before each run). `watching_configured_rules.rs`
writes trigger files at `/tmp/fzz/...` and `examples/tasks-with-absolute-paths.yml`
watches those exact paths. Any two concurrent runs — the Funzzy watcher
generation plus a manual `cargo test --features test-integration`, CI, or a
second watcher generation — clobber each other's watch roots and trigger files,
producing nondeterministic failures (observed: watcher gen=16 integration step
exit 101 while another run was active; clean PASS when runs are sequential).

Related flake observed under parallel load: `tasks_that_run_on_init` asserts a
full-output string around a 5s timing window (see its `FIXME`).

## Scope

- `tests/watching_configured_rules.rs`
- `examples/tasks-with-absolute-paths.yml`
- `tests/common/lib.rs`
- `.watch.yaml` and `Makefile` integration-task scratch prep
- `src/rules.rs` template options (only if adding `{{env:VAR}}`-style expansion)

## Acceptance criteria

- [ ] Two integration runs started concurrently never interfere with each other's scratch.
- [ ] A run's `rm`/prep never destroys another live run's watch roots.
- [ ] Absolute-path and unknown-path behaviors (`@valid`, `@invalid`) remain covered.
- [ ] Sequential runs behave exactly as today (no new flakiness).
- [ ] Watcher and manual invocations share one isolation mechanism.

## Verification

- Concurrent `cargo test --features test-integration` invocations pass deterministically.
- Watcher `run integration` target passes while a manual run is in flight.
- Full default and feature-gated suites pass.
