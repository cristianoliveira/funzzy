---
id: TASK-0166
title: Prevent baseline seeding from following symlink cycles
status: todo
depends_on: []
priority: high
tags: [rust, watcher, filesystem, symlink, startup, reliability, tdd]
---

# Prevent baseline seeding from following symlink cycles

## Problem
A broad watch glob can make startup baseline the whole workspace, and ModificationGate::seed recursively follows directory symlinks without cycle protection. A symlink back to an ancestor causes fzz to consume CPU and memory indefinitely before readiness, so no Watching output or control socket appears.

## Incident evidence

Two `fzz` processes in `/Users/cristianoliveira/work/cells` (PIDs `67032` and `68510`) remained CPU-bound and grew to roughly 1.5 GB RSS each before printing `Watching...` or creating `.watch.sock`.

The generated configuration contains `change: '**/*.txt'`, whose empty literal prefix selects the workspace as a baseline root. Startup calls `ModificationGate::seed()` before readiness. Its recursive `path.is_dir()` traversal follows this cycle:

```text
work/cells/.tmp/docs
  -> other/private
  -> quota/.local/cells
  -> work/cells
```

Canonical investigation handoff: `.tmp/reports/dear-diary/2026-09-03/19-05-31--stuck-fzz-cells--01a06833-96fa-74f4-a281-35f517ce3bd9.md`.

## Desired outcome

Baseline seeding terminates for every finite filesystem graph, including ancestor-pointing directory symlinks, without changing ordinary file modification baselines or delaying watcher readiness indefinitely.

## Acceptance criteria

- [ ] Before changing production code, add a focused `ModificationGate::seed` regression with a directory symlink pointing to an ancestor; prove seeding returns and records ordinary files once.
- [ ] Add the unhappy-path variant for an unreadable, broken, or disappearing symlink/entry without panicking or retrying forever.
- [ ] Make baseline traversal use the established `walk_descendants` policy: record symlink paths when appropriate but never descend into symlinked directories or `.git` directories.
- [ ] Prefer one iterative, symlink-safe traversal policy or a justified shared helper over recursive traversal that can overflow or cycle.
- [ ] Preserve fill-only `last_seen` behavior, file mtimes, disjoint baseline seeding, and first-change routing for normal directories.
- [ ] Add a spawned-watcher regression using a broad-root glob and ancestor symlink cycle; assert a bounded readiness marker/control socket appears without unbounded traversal.
- [ ] Prove watcher shutdown cleans the fixture and leaves no spawned child or background watcher process.
- [ ] Audit the generated starter configuration's broad-root patterns. Either narrow them without reducing the example's purpose, or document the whole-workspace baseline cost and symlink-safe guarantee; lock the decision with a generated-config test.
- [ ] Run focused unit tests, feature-gated filesystem integration, lint/format, and the configured final gate.

## Non-goals

- Following directory symlinks as additional watch roots.
- Adding a generic startup timeout in place of fixing traversal.
- Changing glob matching semantics or the literal-prefix root algorithm.
- Automatically restoring `/Users/cristianoliveira/work/cells/.watch.yaml`.

## Operational precondition

Before validating against the real `work/cells` workspace, terminate PIDs `67032` and `68510` gracefully, confirm they are gone, inspect `.tmp/.watch.yaml` before restoring any configuration, and start only one rebuilt watcher.

## Test constraints

Use bounded synchronization and unique temporary directories. Do not use fixed sleeps as termination proof. Symlink tests must be Unix-gated where required and must clean up even when assertions fail.

