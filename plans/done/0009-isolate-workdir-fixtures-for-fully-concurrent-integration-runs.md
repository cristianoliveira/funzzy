---
id: TASK-0009
title: Isolate workdir fixtures for fully concurrent integration runs
status: done
depends_on: [TASK-0008]
priority: low
tags: [tests, concurrency, reliability]
---

# Isolate workdir fixtures for fully concurrent integration runs

## Problem

Most integration tests share the checked-in `examples/workdir/` tree as a
trigger fixture (tests write `examples/workdir/frontend/test.js` etc.; example
configs watch `examples/workdir/**` relative to the fzz working directory).
Two concurrent integration runs (watcher generation + manual invocation) write
the same files and both fzz processes watch the same paths, so run A sees
run B's triggers. Tests with exact-output or negative assertions
(`watching_nested_groups.rs`, `watching_configured_rules.rs` simple-case) then
fail. TASK-0008 removed the destructive `/tmp/fzz` scratch and log-file
collisions; this task covers the workdir trigger vector.

## Scope

- `examples/workdir/**` fixture usage across `tests/*.rs` and example configs
- `tests/common/lib.rs` (per-run fixture root, e.g. `temp_dir()/funzzy-fixture-<pid>`)
- A way for generated configs and test writes to share one per-run root
  (config template substitution, similar to TASK-0008's `scratch_config`)

## Acceptance criteria

- [x] Two full integration suites run concurrently pass deterministically.
- [x] Per-run workdir fixtures never share files with another run.
- [x] Relative-path glob behavior (`examples/workdir/**/*`) remains covered.
- [x] Sequential runs behave exactly as today.
- [x] No shared mutable fixture remains in `examples/workdir` for tests.

## Verification

- Two concurrent `cargo test --features test-integration` invocations pass.
- Watcher `run integration` target passes while a manual run is in flight.
- Full default and feature-gated suites pass.
