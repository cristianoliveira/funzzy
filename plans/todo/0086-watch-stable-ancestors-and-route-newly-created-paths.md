---
id: TASK-0086
title: Watch stable ancestors and route newly created paths
status: todo
depends_on: [TASK-0085, TASK-0031, TASK-0022]
priority: high
tags: [rust, watcher, notify, polling, filesystem, tdd]
---

# Watch stable ancestors and route newly created paths

## Problem
Native and polling backends need one deterministic root-planning strategy that watches nearest existing ancestors, discovers future descendants, and applies normal matching/ignore/batching exactly once.

## Context

Centralize pure subscription-root planning in `Watches`; keep `watcher.rs` as backend adapter and `watch_loop.rs` as one routing flow.

## Acceptance criteria

- [ ] Pure tests first derive minimal deterministic nearest-existing ancestor roots from exact files, globs, missing nested prefixes, absolute/relative paths, and overlapping patterns.
- [ ] Root set is canonicalized, deduplicated, containment-minimized, workspace-bounded, and stable independent of hash/map order.
- [ ] Native notify registers recursive stable ancestors and surfaces create/rename/remove events with canonical paths through existing batch normalization.
- [ ] Poll scanner discovers additions/removals recursively according to same bounded roots and does not emit baseline contents as changes.
- [ ] Newly created matching path reaches `Watches` selection/ignore/gitignore exactly once per normalized batch; no separate create execution path.
- [ ] Directory creation updates coverage without rebuilding watcher process; delete/recreate does not leave stale subscription assumptions.
- [ ] Atomic rename uses final destination for matching where backend supplies it; duplicate temp/final events are deterministically deduped within configured debounce.
- [ ] Config reload atomically replaces root plan and old roots stop routing after instance boundary.
- [ ] Root watching does not traverse symlink cycles, `.git`, state/socket/log outputs, or ignored trees contrary to contract and stays resource bounded.
- [ ] Verbose/explain diagnostics name pattern→subscription-root decision and why missing future prefix is covered.

## Notes
