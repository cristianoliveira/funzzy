---
id: TASK-0114
title: Stop directory-path events from routing and escaping ignore globs
status: doing
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
- [ ] Criterion 1
- [ ] Criterion 2

## Notes

