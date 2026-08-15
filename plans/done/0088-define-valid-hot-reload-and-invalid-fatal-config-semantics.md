---
id: TASK-0088
title: Define valid hot reload and invalid fatal config semantics
status: done
depends_on: [TASK-0033, TASK-0043, TASK-0085]
priority: high
tags: [design, watcher, config, reload, lifecycle, determinism]
---

# Define valid hot reload and invalid fatal config semantics

## Problem
Funzzy currently sends SIGTERM to its own PID for every real .watch.yaml edit, so even valid changes destroy watcher continuity; invalid replacements, however, should fail fast rather than silently keep stale behavior.

## Context

This concerns Funzzy watcher itself, not pi-watcher. Replace unconditional self-SIGTERM with validate-first branch:

- valid and operationally preparable candidate → hot reload in same process;
- invalid/unpreparable candidate → deterministic graceful fatal shutdown with nonzero exit.

## Acceptance criteria

Contract document: `docs/CONFIG-RELOAD-CONTRACT.md` (normative, defined by TASK-0088).

- [x] Contract defines syntactic/schema/semantic/operational validity and exact point candidate becomes live. (§1 four gates in order; live point = atomic commit.)
- [x] Config save is debounced and stable-read/atomic-rename aware so transient partial writes are not misclassified before validation window closes. (§2 debounced batches, mtime-stable read, atomic-rename/delete-recreate canonicalization, baseline guard.)
- [x] Valid candidate preserves watcher PID/process, instance token, control service, monotonic batch/generation IDs, retained output, and active subscriptions. (§3 continuity list.)
- [x] Invalid YAML/schema/job/pattern/value or candidate resource preparation failure never leaves old config running silently: emit terminal config error, cancel/reap owned children/services, close resources, and exit nonzero. (§5 graceful fatal shutdown.)
- [x] Fatal shutdown is coordinated through process ownership; no SIGKILL/panic/self-SIGTERM shortcut and no orphan descendants. (§5 via `process_owner::shutdown_all`; same path as Ctrl-C.)
- [x] Active generation freezes old valid revision; valid reload does not retroactively mutate its plan/outcome/output. (§4 revision boundary.)
- [x] Events accepted after atomic commit use new revision; ordering at commit boundary is deterministic and observable. (§4 deterministic commit ordering, duplicate normalization once.)
- [x] Added/removed jobs, roots, ignore/gitignore, concurrency, debounce, hooks/output policies, sequential defaults, services, backend, and control socket changes are classified with reload strategy. (§6 classification table: swap / prepare-gated / out-of-scope-fatal.)
- [x] Every schema-valid setting either has safe in-process swap semantics or fails candidate preparation and takes invalid fatal path; no undocumented restart-required middle state. (§6 out-of-scope row + §5 bounded validation.)
- [x] Config deletion/recreation and editor temporary rename behavior are explicit. (§7 deletion → fatal; recreation → fresh candidate; temp files never trigger.)

## Notes

“Kill on invalid” means graceful fatal watcher shutdown after bounded validation, not abrupt process signal that skips cleanup.
