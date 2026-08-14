---
id: TASK-0031
title: Make filesystem event batching configurable
status: done
depends_on: [TASK-0022, TASK-0023]
priority: high
tags: [rust, watcher, debounce, events, determinism, tdd]
---

# Make filesystem event batching configurable

## Problem
Funzzy hardcodes a one-second debounce and reduces a batch to individual path handling, limiting responsiveness, observability, and workflows that need the complete changed-path set.

## Context

Replace hardcoded watcher debounce with domain `EventBatch` policy. Keep native notify details in adapter.

## Acceptance criteria

- [ ] Fake-clock tests define trailing-edge debounce, deduplication, normalized ordering, maximum wait, cancellation, and shutdown flush behavior.
- [ ] `on.debounce` accepts documented duration syntax and rejects zero/invalid values.
- [ ] Existing one-second behavior remains default unless contract intentionally changes it.
- [ ] One batch preserves all normalized changed paths and event kinds available from backend.
- [ ] Matching runs once per batch, not once per duplicate backend event.
- [ ] Templates expose backward-compatible `{{filepath}}` plus explicit batch value such as `{{paths}}` with safe escaping contract.
- [ ] Verbose diagnostics show batch ID, size, paths, and collapse reason deterministically.
- [ ] Synthetic `control emit` routing policy relative to debounce is documented and tested.

## Notes

