---
id: TASK-0116
title: Define the V2 configuration information architecture
status: todo
depends_on: []
priority: high
tags: [design, config, v2, ux, migration]
---

# Define the V2 configuration information architecture

## Problem
The current `on` section mixes event inputs, execution policy, and lifecycle reactions, so users cannot predict where configuration belongs.

## Context

Define config format versions independently from the Funzzy binary version. Proposed canonical V2 direction:

```yaml
on:
  change: "src/**"
  socket: .tmp/funzzy/control.sock
  debounce: 500ms

execution:
  concurrency: 2
  output: show-on-failure

hooks:
  success: echo ok
  failure: echo failed
  close: echo closed

jobs:
  - name: test
    run: cargo test
```

`on` owns input-event configuration and processing (`change`, `ignore`, `socket`, debounce/backend/gitignore policy). `execution` owns scheduling and output policy. `hooks` owns lifecycle reactions. `jobs` remains ordered work.

## Acceptance criteria

- [ ] Publish a normative V2 configuration contract with the directional model: events enter through `on`, jobs run under `execution`, lifecycle reactions run through `hooks`.
- [ ] Define canonical V2 structurally without adding a top-level version property; keep the established V1 task-list compatibility boundary separate from this section reorganization.
- [ ] Assign every currently accepted preferred property to exactly one V2 owner; no property is silently dropped.
- [ ] Keep `on` focused on event inputs/processing, including the control socket as an input source.
- [ ] Move `on.concurrency` and `on.output` to `execution.concurrency` and `execution.output`.
- [ ] Move `on.success`, `on.failure`, and `on.close` to `hooks` without changing their lifecycle/result semantics.
- [ ] Define defaults, unknown-property behavior, hot-reload semantics, and field-path validation errors for each new section.
- [ ] Decide and document whether pre-existing grouped V2 field placements remain parser aliases or become a manual breaking edit; they are explicitly not inputs to `fzz migrate`.
- [ ] Preserve the established migration boundary: V1 task-list vocabulary to V2 ordered `jobs`; it is not a formatter or a mechanism for reorganizing V2 sections.
- [ ] Record compatibility impact on the control socket, pi-watcher, run events, and job semantics; these remain unchanged unless explicitly listed.

## Notes

This task locks product vocabulary before parser, schema, generators, migration, or documentation changes begin.
