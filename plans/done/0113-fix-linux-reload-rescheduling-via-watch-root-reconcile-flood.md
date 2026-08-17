---
id: TASK-0113
title: Fix Linux reload rescheduling via watch-root reconcile flood
status: done
depends_on: []
priority: high
tags: []
---

# Fix Linux reload rescheduling via watch-root reconcile flood

## Problem
On Linux (inotify), notify emits bookkeeping Any-events on the watched root directory itself around config reloads. reconcile_new_directories treats any non-continuous directory event as a newly created directory and walks its whole subtree, synthesizing pre-existing files into the batch. After a reload that batch routes under the new revision and supersedes busy generations — breaking the reload contract (busy old-revision generation must complete) and causing CI failures in config_reload_lifecycle/config_reload_matrix on Linux only (macOS gates pass).

## Context
(Optional: approach, links, related tasks.)

## Acceptance criteria
- [x] Criterion 1
- [x] Criterion 2

## Notes


## Diagnosis (Linux container, instrumented)

CI failures at 1121f8a (config_reload_lifecycle busy_old_revision + 3 matrix tests) reproduce only on Linux (inotify); macOS passes 3x at the same commit. Kernel inotifywait shows NO events at reload time — the spurious batches are notify-internal bookkeeping `Any` events on the WATCHED ROOT directory itself. `reconcile_new_directories` (WATCH-DISCOVERY-CONTRACT §4) treats any non-continuous directory event as a newly-created directory and walks the whole subtree, synthesizing pre-existing files (e.g. src/a.rs) into the batch. Post-reload that batch routes under the NEW revision and supersedes the busy generation (restart policy implied by control socket).

## Fix

A path equal to an ACTIVE WATCH ROOT is a backend self-event, never a newly created directory: skip the rescan for it (other directories keep the §4 rescan — a genuinely new directory arrives as a path under a root, never equal to it). `reconcile_new_directories(events, watch_roots)`; native loop passes `current_roots`.

## Evidence

- New unit tests: `watch_root_bookkeeping_event_never_rescans_its_subtree` (red first), `new_directory_under_a_root_still_rescans_after_the_fix`; 3 existing reconcile tests updated for the new parameter (same scenarios).
- Linux container (rust:1.97-slim): manual busy-generation scenario now completes (slow-verdict survives reload); config_reload_lifecycle 14/14 PASS; config_reload_matrix 6/7 — remaining failure (`ignore_rule_added_on_reload_stops_routing_ignored_path`) is a SEPARATE pre-existing Linux gap tracked as TASK-0114.

## Outcome

Fix landed in 45e984f. Diagnosed in a Linux container via kernel inotify tracing: the spurious post-reload batches are notify-internal bookkeeping Any-events on the WATCHED ROOT itself (kernel emits nothing); reconcile_new_directories walked the root subtree and re-routed pre-existing files under the new revision, superseding busy generations. Root self-events no longer rescan. Superseded in part by TASK-0114 (root/directory paths now dropped from batches entirely).
