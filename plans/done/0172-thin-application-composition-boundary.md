---
id: TASK-0172
title: Thin application composition and CLI boundary
status: done
depends_on: [TASK-0170, TASK-0171]
priority: normal
tags: [architecture, app, cli, composition-root]
---

# Thin application composition and CLI boundary

## Problem

`src/app.rs` mixes CLI dispatch, config discovery, runtime setup, signal handling, and watch execution. This composition-root complexity encourages domain decisions to leak into CLI and runtime code.

## Desired outcome

Keep `app::run` as a small composition root that translates CLI input into domain commands and wires infrastructure ports. Domain modules remain unaware of CLI types and process/runtime setup.

## Acceptance criteria

- [x] Extract private command handlers without moving domain decisions into CLI modules or creating generic utility abstractions.
- [x] Pass explicit settings/port bundles across the composition boundary; avoid new boolean flag combinations and global lookups.
- [x] Prove domain modules do not import CLI, stdout/logging, watcher/runtime, process, filesystem, or control-socket modules.
- [x] Preserve CLI precedence, exit codes, config discovery, reload readiness, signal shutdown, and blocking/non-blocking watch behavior.
- [x] Add boundary tests that construct domain commands without parsing argv or starting a watcher.

## Verification

Run argument, CLI, init, watch, reload, signal/shutdown, and feature-gated integration tests; run `make lint`; re-run module/SOLID/DI/complexity scans.

## Evidence

- Handlers: `Startup::from_args` bundles diagnostics/log/event-stream/workspace-root (`999e62a`); `watch_action`, `run_target_action`, `explain_action`, `exec_action` extracted with `run()` as pure dispatch (`08b76b0`, `ccc4a7d`). Small commands already delegate through `src/cli` `Command` trait.
- Bundles: handlers take `&Arguments` + `&Startup`; no new booleans or global lookups introduced.
- Domain purity: `cargo test --test domain_boundaries` — 8 passed after the change.
- Behavior: `cli_arguments` 53, `exec_argv` 5, `run_once` 19, `watching_with_log_file` 2, `config_reload_lifecycle` 7, `watch_exclusions` 2 passed; watcher gen135 full unit gate passed fresh; watcher gen137 integration gate passed on unchanged fingerprint `9ae1c747ce47`.
- Lint/fmt: `make lint` clean after `ccc4a7d`; `cargo fmt -- --check` clean.
- Scans: app.rs complexity top-5 average 2.6, zero high-complexity functions; SOLID scan recorded at 18 findings for the composition-root adapter (pre-existing composition-root scope, not domain code).
- Boundary tests: `app::tests` construct `Watches` and exercise `explain_output`/`literal_prefix` directly — no argv parsing, no watcher startup (3 passed).

