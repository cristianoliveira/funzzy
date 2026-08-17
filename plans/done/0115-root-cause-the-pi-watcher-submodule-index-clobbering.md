---
id: TASK-0115
title: Root-cause the pi-watcher submodule index clobbering
status: done
depends_on: []
priority: high
tags: []
---

# Root-cause the pi-watcher submodule index clobbering

## Problem
Twice now (Aug 16 parallel session, and TASK-0106 this session) the pi-watcher submodule index .git/modules/pi-watcher/index gets replaced by a junk 2-entry index (.watch.yaml + src/main.ts pointing at missing blobs) with an invalid cache-tree, breaking commits until repaired via read-tree HEAD. The writer is unidentified; both sessions worked around it non-destructively, and it will keep corrupting work until fixed.

## Context
(Optional: approach, links, related tasks.)

## Acceptance criteria
- [ ] Criterion 1
- [ ] Criterion 2

## Notes


## Root cause + fix (09-04-26, DONE)

**Mechanism (reproduced byte-exact):** committing inside the pi-watcher submodule makes git export `GIT_DIR=<root>/.git/modules/pi-watcher` (+ GIT_WORK_TREE/GIT_INDEX_FILE) into the pre-commit hook. Any `git` subprocess launched from that hook's process tree without env sanitization, run with `cwd` = a scratch worktree, redirects to the SUBMODULE repo: `git add -A` there writes the submodule index with the scratch worktree's files. The e2e `createWorktree` fixture (.watch.yaml + src/main.ts, deterministic content) produces exactly the observed junk 2-entry index, including the recurring missing blob `749aa994…` (hash of the deterministic `export const version = 1;` line). Live demo: `env GIT_DIR=…/modules/pi-watcher GIT_WORK_TREE=<tmp> git add -A` → junk index restored exactly.

**Prior state:** test-side `gitEnv()` sanitization (committed `da02139`, Aug 16) covers known suite spawns — full-suite runs under simulated hook env leave the index untouched (verified twice). Today's 18:31 recurrence therefore came from an unsanitized git invocation during the hook window outside the covered suite paths (concurrent live session most plausible; not isolatable post-hoc).

**Fix (defense in depth, at the leak source):** `.githooks/pre-commit` and `.githooks/pre-push` now `unset GIT_DIR GIT_WORK_TREE GIT_INDEX_FILE GIT_OBJECT_DIRECTORY` before `make` — hook-launched processes can no longer inherit the redirect regardless of spawn-site sanitization. pi-watcher commit `815b887` (made THROUGH the full pre-commit hook, 452/452 green, index valid 138 entries, fsck clean — the exact flow that failed twice on Aug 16–17).

**Residual (documented):** git commands run by humans/agents outside hooks with manually leaked env are out of repo control; the Aug-16 `index.bak-*` and today's incidents are consistent with the hook leak class. Non-destructive repair if it ever recurs: `git read-tree HEAD` in the submodule.
