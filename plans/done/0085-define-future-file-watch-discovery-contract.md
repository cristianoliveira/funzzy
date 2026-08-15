---
id: TASK-0085
title: Define future-file watch discovery contract
status: done
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

Contract document: `docs/WATCH-DISCOVERY-CONTRACT.md` (normative, defined by TASK-0085).

- [x] Contract distinguishes configured patterns, watch subscription roots, observed filesystem paths, matched jobs, and Git tracking. (§1 vocabulary table + rules.)
- [x] Creating file under existing watched directory is observed and routed through same normalize→ignore→match→batch→busy-policy flow as modification. (§2 uniform flow; no separate create path; event kind never changes matching.)
- [x] Pattern whose literal directory prefix is partly/nonexistent at startup watches nearest existing ancestor and begins matching when missing descendants appear. (§3 nearest-existing-ancestor rule; root fallback bounded.)
- [x] Creating directory tree and file in one operation eventually yields canonical final path; intermediate directory events do not run unrelated jobs. (§4 tree-creation paragraph; deterministic first-match batch routing.)
- [x] Atomic editor save (temp create/write/rename over destination) triggers destination semantics once per debounce batch; temp/ignored path does not leak as selected job. (§4 atomic-save paragraph; dedup within debounce, rename destination preferred, ignore step drops temp names.)
- [x] Delete then recreate file/directory remains observable without watcher restart. (§5 stable-ancestor subscription; poll existence-change detection; no stale assumptions.)
- [x] Per-job ignore, global ignore, gitignore precedence, symlink policy, workspace escape, hidden paths, and config-file reload interactions are explicit. (§6 precedence list + explicit policies; aligns with GITIGNORE-CONTRACT §1.)
- [x] Native and polling backends promise equivalent matched-path outcome; raw event counts/order are not contractual. (§7 backend equivalence; poll baseline seeding; identical routing flow.)
- [x] Startup/explain diagnostics show stable subscription roots and future-path coverage; truly unwatchable roots fail/warn actionably rather than silently miss. (§8 watch_root records, explain coverage naming, actionable warn/error split — matches current `src/watcher.rs` behavior.)
- [x] Synthetic `emit` remains routing-equivalent for nonexistent/future path but does not claim filesystem subscription proof. (§9 emit routes through the same matching policy; never asserts subscription coverage.)

## Notes

Do not recursively watch workspace root by default when narrower safe ancestor exists; bound resource/cycle behavior.
