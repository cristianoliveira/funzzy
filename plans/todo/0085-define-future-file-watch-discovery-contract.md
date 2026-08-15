---
id: TASK-0085
title: Define future-file watch discovery contract
status: todo
depends_on: [TASK-0014, TASK-0019, TASK-0036, TASK-0037]
priority: high
tags: [design, watcher, filesystem, create, matching, determinism]
---

# Define future-file watch discovery contract

## Problem
Users configure path patterns before all files/directories exist, but a watcher that subscribes only to startup-resolved paths can silently miss later creations and give no feedback that configured future work is unwatched.

## Context

“Tracked” means observed and routed by Funzzy, not staged through Git. Configuration may name glob roots/directories that do not exist at watcher startup.

## User story

**As a** Funzzy user editing a watched workspace
**I want** files and directories created after watcher startup to enter normal matching automatically
**So that** adding source/tests/configuration triggers expected jobs without restarting watcher or touching existing files.

## Acceptance criteria

- [ ] Contract distinguishes configured patterns, watch subscription roots, observed filesystem paths, matched jobs, and Git tracking.
- [ ] Creating file under existing watched directory is observed and routed through same normalize→ignore→match→batch→busy-policy flow as modification.
- [ ] Pattern whose literal directory prefix is partly/nonexistent at startup watches nearest existing ancestor and begins matching when missing descendants appear.
- [ ] Creating directory tree and file in one operation eventually yields canonical final path; intermediate directory events do not run unrelated jobs.
- [ ] Atomic editor save (temp create/write/rename over destination) triggers destination semantics once per debounce batch; temp/ignored path does not leak as selected job.
- [ ] Delete then recreate file/directory remains observable without watcher restart.
- [ ] Per-job ignore, global ignore, gitignore precedence, symlink policy, workspace escape, hidden paths, and config-file reload interactions are explicit.
- [ ] Native and polling backends promise equivalent matched-path outcome; raw event counts/order are not contractual.
- [ ] Startup/explain diagnostics show stable subscription roots and future-path coverage; truly unwatchable roots fail/warn actionably rather than silently miss.
- [ ] Synthetic `emit` remains routing-equivalent for nonexistent/future path but does not claim filesystem subscription proof.

## Notes

Do not recursively watch workspace root by default when narrower safe ancestor exists; bound resource/cycle behavior.
