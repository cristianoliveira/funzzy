---
id: TASK-0172
title: Thin application composition and CLI boundary
status: doing
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

- [ ] Extract private command handlers without moving domain decisions into CLI modules or creating generic utility abstractions.
- [ ] Pass explicit settings/port bundles across the composition boundary; avoid new boolean flag combinations and global lookups.
- [ ] Prove domain modules do not import CLI, stdout/logging, watcher/runtime, process, filesystem, or control-socket modules.
- [ ] Preserve CLI precedence, exit codes, config discovery, reload readiness, signal shutdown, and blocking/non-blocking watch behavior.
- [ ] Add boundary tests that construct domain commands without parsing argv or starting a watcher.

## Verification

Run argument, CLI, init, watch, reload, signal/shutdown, and feature-gated integration tests; run `make lint`; re-run module/SOLID/DI/complexity scans.
