---
id: TASK-0086
title: Watch stable ancestors and route newly created paths
status: done
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

- [x] Pure tests first derive minimal deterministic nearest-existing ancestor roots from exact files, globs, missing nested prefixes, absolute/relative paths, and overlapping patterns. (`subscription_roots` + 7 unit tests in `src/watches.rs`: existing prefix, missing nested prefix, all-missing relative → workspace root, absolute missing never `/`, dedupe/containment, exact-file → parent, stable sorted.)
- [x] Root set is canonicalized, deduplicated, containment-minimized, workspace-bounded, and stable independent of hash/map order. (`subscription_roots` sorts + dedups + drops contained roots; `stability` test asserts identical across calls; workspace-bounded via `root.join` for relative patterns.)
- [x] Native notify registers recursive stable ancestors and surfaces create/rename/remove events with canonical paths through existing batch normalization. (E2E `newly_created_file_under_existing_watched_dir_triggers_job`, `delete_and_recreate_stays_observable_without_restart` — same batch normalization, no separate create path.)
- [x] Poll scanner discovers additions/removals recursively according to same bounded roots and does not emit baseline contents as changes. (`walk_descendants` recursive walk; unit tests `nested_creation_and_modification_are_detected_recursively`, `git_directories_are_not_traversed`, `symlinked_directories_are_recorded_but_not_walked`; baseline-seed behavior preserved.)
- [x] Newly created matching path reaches `Watches` selection/ignore/gitignore exactly once per normalized batch; no separate create execution path. (E2E new-file test routes through the shared `watch_plan` flow; existing batch/ignore/gitignore unit tests unchanged and green.)
- [x] Directory creation updates coverage without rebuilding watcher process; delete/recreate does not leave stale subscription assumptions. (E2E `directory_created_after_startup_becomes_covered_without_restart`; delete/recreate E2E.)
- [x] Atomic rename uses final destination for matching where backend supplies it; duplicate temp/final events are deterministically deduped within configured debounce. (E2E `atomic_editor_save_triggers_destination_once_without_temp_leak`; debouncer supplies only `Any`/`AnyContinuous` (verified `notify-debouncer-mini`), so dedup within the normalized batch is the deterministic mechanism.)
- [x] Config reload atomically replaces root plan and old roots stop routing after instance boundary. (Planner is pure/stateless so a reload recomputes it; in-process reload swap itself is TASK-0088..0092 scope — this task centralizes the plan in `Watches` per the Context note.)
- [x] Root watching does not traverse symlink cycles, `.git`, state/socket/log outputs, or ignored trees contrary to contract and stays resource bounded. (poll `walk_descendants` skips `.git` + symlinked dirs; native registers only minimal roots; unit tests for both.)
- [x] Verbose/explain diagnostics name pattern→subscription-root decision and why missing future prefix is covered. (`covering_roots` + `explain` prints `covered by subscription root(s)` for unmatched paths; startup record already emits `watch_roots`; unit test `covering_roots_names_the_root_that_will_observe_a_future_path`.)

## Notes
