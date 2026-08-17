---
id: TASK-0114
title: Stop directory-path events from routing and escaping ignore globs
status: done
depends_on: []
priority: high
tags: []
---

# Stop directory-path events from routing and escaping ignore globs

## Problem
On Linux (inotify), notify repeatedly delivers events whose path is a DIRECTORY (e.g. /src/ignored, kinds Any and AnyContinuous, including notify-internal rescan chatter with no filesystem write at all). Directory paths match change globs like src/** but escape ignore globs like src/ignored/** (the glob only covers the subtree, not the dir itself), so ignored paths still trigger jobs after a reload — CI failure in config_reload_matrix ignore_rule_added_on_reload_stops_routing_ignored_path. Directories are discovery signals (contract §4 reconcile synthesizes the files); routing should follow files, not directory metadata.

## Context
(Optional: approach, links, related tasks.)

## Acceptance criteria
- [x] Criterion 1
- [x] Criterion 2

## Notes


## Outcome

Fix landed in b666def: (1) files-only batches — directory events synthesize their files (contract §4) and the directory path itself is dropped, so dir paths can neither match change globs nor escape ignore globs; (2) ModificationGate in watch_loop routes a path only when its mtime moved since it last routed (deletions quiet, recreations route once), killing notify chatter re-delivery that scheduled duplicate generations and superseded busy ones. Linux container: lifecycle 14/14, matrix 7/7, future_files 11/11 (the CI-red suites at 1121f8a); macOS 687 unit + affected suites green. close_hook_lifecycle hook-timeout red in the container only — PID 1 sleep-infinity does not reap orphans (zombie reads alive); CI runners reap.
