---
id: TASK-0092
title: Prove valid reload continuity and invalid config shutdown
status: todo
depends_on: [TASK-0090, TASK-0091, TASK-0087, TASK-0030]
priority: high
tags: [integration-tests, watcher, config, reload, lifecycle, reliability]
---

# Prove valid reload continuity and invalid config shutdown

## Problem
Unit swaps cannot prove real atomic editor writes, busy generations, socket subscriptions, root changes, managed services, invalid YAML, and descendant cleanup follow the intended split behavior.

## Context

Use real process/control socket and deterministic barriers; assert PID/instance/revision/process ownership rather than timing guesses.

## Acceptance criteria

- [ ] Valid atomic config rewrite keeps same PID and instance token, increments revision once, preserves subscriber, and new matching job runs without watcher restart.
- [ ] Formatting/comment-only rewrite is no-op; rapid valid writes debounce to final candidate and do not create mixed/intermediate revisions.
- [ ] Busy old-revision generation completes with original jobs while subsequent event runs new revision; outputs/estimates/references identify each correctly.
- [ ] Root added for initially missing future file observes later create; removed root stops routing after commit without duplicate boundary generation.
- [ ] Concurrency/debounce/ignore/hook/output/service/backend/socket valid changes follow TASK-0090 transaction and preserve process.
- [ ] Invalid YAML, schema, semantic job, unwatchable root/backend, and occupied new socket each produce deterministic error, graceful child/service cleanup, socket closure, and nonzero process exit.
- [ ] Partial editor write inside stable-read/debounce window followed by valid final content does not spuriously exit; content remaining invalid after window does.
- [ ] Delete/recreate config policy is proven and does not spin/reload repeatedly.
- [ ] Ctrl-C during prepare/commit and invalid shutdown leaves no descendants, subscriptions, sockets, roots, or temp state.
- [ ] Regression test fails against current unconditional self-SIGTERM implementation and old config-reload instance-change assertions are updated.
- [ ] User docs distinguish valid hot reload, invalid fatal exit, true external restart, and managed-service replacement.

## Notes
