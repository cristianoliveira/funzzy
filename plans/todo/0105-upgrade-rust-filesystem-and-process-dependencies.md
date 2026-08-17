---
id: TASK-0105
title: Upgrade Rust filesystem and process dependencies
status: doing
depends_on: [TASK-0104]
priority: high
tags: [rust, cargo, notify, nix, watcher, signals, integration-tests]
---

# Upgrade Rust filesystem and process dependencies

## Problem
The watcher and signal stack uses old notify, debouncer, and nix APIs—including duplicate notify major versions—so major upgrades need isolated behavioral proof rather than a bulk lockfile bump.

## Context

Upgrade in two explicit checkpoints: filesystem notification stack, then Unix process/signal stack. These libraries sit on compatibility surfaces covered by WATCH-DISCOVERY-CONTRACT and process-group shutdown tests.

## Acceptance criteria

- [ ] Write/confirm failing characterization coverage first for native create/modify/remove/rename, future nested directories, root swaps, backend fallback, debounce batches, SIGINT/SIGTERM, restart cancellation, and descendant reaping.
- [ ] Upgrade `notify-debouncer-mini` from 0.3 to policy-approved current stable and align on one notify major; remove obsolete direct `notify 4` if source only uses debouncer re-export.
- [ ] Adapt event/error/root registration APIs in watcher adapter only; matching, batching, busy policy, and workflow layers remain dependency-agnostic.
- [ ] Preserve native and poll equivalent matched-path outcomes, including files created inside newly created directories.
- [ ] Upgrade `nix` from 0.26 to approved current stable with minimal required features instead of broad defaults.
- [ ] Preserve async-signal-safe handler behavior, process-group creation, cancellation grace/escalation, conventional 130/143 exits, and no orphan descendants.
- [ ] Feature/cfg behavior remains explicit for supported Unix platforms; unsupported platforms do not gain silent degraded process handling.
- [ ] Dependency tree contains one notify major and no avoidable old fsevent/bitflags chain.
- [ ] Focused watcher, future-files, config-reload, process-groups, run-once cancellation, and non-block integration tests pass without loosening timeouts/assertions.
- [ ] Any unavoidable upstream behavior difference is documented in contract and changelog before acceptance.

## Notes

Failures are information; do not mask backend changes with sleeps or event-count assumptions.

