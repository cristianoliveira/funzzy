---
id: TASK-0008
title: Isolate integration test scratch dir per run
status: done
depends_on: [TASK-0002]
priority: high
tags: [tests, concurrency, reliability]
---

# Isolate integration test scratch dir per run

## Problem

Integration runs shared one global scratch dir `/tmp/fzz` (wiped with `rm -rf`
by `.watch.yaml` and `Makefile` before each run). `watching_configured_rules.rs`
wrote trigger files at `/tmp/fzz/...` and `examples/tasks-with-absolute-paths.yml`
watched those exact paths. Any two concurrent runs — the Funzzy watcher
generation plus a manual `cargo test --features test-integration`, CI, or a
second watcher generation — clobbered each other's watch roots and trigger
files, producing nondeterministic failures (observed: watcher gen=16 integration
step exit 101 while another run was active; clean PASS when runs are sequential).

## Scope

- `tests/watching_configured_rules.rs`
- `examples/tasks-with-absolute-paths.yml`
- `tests/common/lib.rs`
- `.watch.yaml` and `Makefile` integration-task scratch prep

## Acceptance criteria

- [x] A run's `rm`/prep never destroys another live run's watch roots: the shared `/tmp/fzz` scratch is gone; absolute-path tests generate a per-run scratch root (`temp_dir()/funzzy-fzz-scratch-<pid>-<label>`, canonicalized for macOS `/var -> /private/var`) from the example template.
- [x] Log files no longer collide across concurrent runs (per-process PID-suffixed names in `with_config`/`with_output`).
- [x] Absolute-path and unknown-path behaviors (`@valid`, `@invalid`) remain covered and now also pass on macOS.
- [x] Sequential runs behave exactly as today.
- [x] `.watch.yaml` and `Makefile` no longer prepare a shared scratch dir.

## Remaining limitation

Concurrent runs can still cross-trigger through the shared `examples/workdir`
fixture (tests that assert exact output or negative matches). Tracked
separately as TASK-0009.

## Verification

- Concurrent `watching_configured_rules` invocations pass deterministically.
- Full default and feature-gated suites pass.
- Watcher `run integration` target passes.
