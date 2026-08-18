---
id: TASK-0120
title: Rewrite configuration and migration documentation for V2
status: todo
depends_on: [TASK-0116, TASK-0118, TASK-0119]
priority: high
tags: [docs, config, migration, init, examples, v2]
---

# Rewrite configuration and migration documentation for V2

## Problem
Users need one coherent explanation of event inputs, execution policy, hooks, and the exact V1-to-V2 migration path.

## Context

Installed CLI discovery is canonical, but repository docs must accurately teach concepts and link users to executable schema/examples. Update normative contracts and user-facing guides together.

## Acceptance criteria

- [ ] Rewrite README configuration examples and terminology around `on`, `execution`, `hooks`, and ordered `jobs`.
- [ ] Update `AGENT-CONFIG-CONTRACT`, run-hooks, init-template, jobs-config, CLI, release/migration, and any other normative documents that name old field paths.
- [ ] Add a concise configuration reference table assigning every V2 property to one section.
- [ ] Explain that `on` configures input events/event processing, including filesystem changes and the control socket.
- [ ] Explain hook timing and result behavior separately for `success`, `failure`, and `close`.
- [ ] Document `fzz init` and every `fzz config example` profile using current generated output rather than hand-maintained divergent YAML.
- [ ] Document the exact V1 task-list migration flow: back up/commit config, run `fzz migrate`, inspect the `jobs:` rewrite, run `fzz check`, then list/run/watch.
- [ ] Document the V2 section reorganization separately from V1 migration; do not imply `fzz migrate` moves execution or hook fields.
- [ ] State explicitly that `fzz migrate` only wraps a V1 root task list or renames root `tasks:` to `jobs:`; it does not format or reorganize V2 configuration.
- [ ] Replace all active references to `on.concurrency`, `on.output`, `on.success`, `on.failure`, and `on.close`; historical plans may remain unchanged.
- [ ] Ensure examples use real commands and field paths verified by CLI tests.

## Notes

Prefer links to `fzz config schema` and `fzz config example` over duplicating full generated artifacts.
