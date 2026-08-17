---
id: TASK-0106
title: Update pi-watcher Node and Pi dependencies
status: done
depends_on: [TASK-0103]
priority: high
tags: [typescript, npm, pi-watcher, dependencies, pi-sdk, security]
---

# Update pi-watcher Node and Pi dependencies

## Problem
The pi-watcher submodule has newer Pi SDK and TypeBox releases available, while a TypeScript major is visible and must be evaluated explicitly instead of accepted transitively.

## Context

Work inside pi-watcher repository, respecting its own branch and clean-state policy. Peer ranges remain intentionally broad only where package consumers provide runtime Pi modules; dev pins prove exact supported integration.

## Acceptance criteria

- [x] Capture `npm outdated`, `npm audit`, package tree, `npm pack --dry-run`, and `make all` baseline before manifest changes.
- [x] Update `@earendil-works/pi-ai` and `@earendil-works/pi-coding-agent` dev pins together to same tested Pi release; verify public extension APIs and TUI/tool registration compile.
- [x] Update TypeBox and YAML within compatible stable lines and confirm runtime decoders still accept additive Funzzy protocol fields and reject malformed payloads.
- [x] Evaluate TypeScript 7 separately against Node 24, types/node, ESLint/typescript-eslint, Vitest/coverage, Pi packages, and current tsconfig; upgrade only as coherent toolchain or document deferral.
- [x] Refresh remaining lint/format/test dependencies only to mutually supported versions; avoid `--force` or legacy peer resolution.
- [x] Keep runtime `dependencies`, Pi `peerDependencies`, and test/build `devDependencies` correctly classified; no Pi runtime package is accidentally bundled.
- [x] `npm ci` reproduces package-lock from clean checkout and `npm audit --audit-level=high` remains green.
- [x] `make quick`, `make all`, real-socket e2e, and `npm pack --dry-run` pass; package file list and tool names remain unchanged unless deliberate.
- [x] Commit extension changes in pi-watcher repository, then update root submodule gitlink explicitly with compatibility evidence.

## Notes

Current pi-watcher worktree is ahead of origin; reconcile ownership/current work before changing it and never overwrite another agent’s changes.


## Outcome (TASK-0106 done)

- pi-watcher commit `e4e40b6`, root gitlink updated in `fa22a92`.
- Updates: pi-ai + pi-coding-agent 0.84.1→0.84.2 (paired), typebox 1.3.7→1.3.15.
  yaml 2.9.0 already current; all other devDeps at latest supported (verified via npm outdated Wanted column).
- TypeScript 7.0.2 DEFERRED: typescript-eslint 8.67.0 peer range is `typescript >=4.8.4 <6.1.0`; TS 7 would break lint — revisit when typescript-eslint supports it.
- Evidence: baseline + post logs in `funzzy/.tmp/piwatcher-0106/`; make quick/make all 452/452 incl. real-socket e2e and fail-closed malformed-payload decoder tests; npm pack surface identical (21 files / 28.0 kB); npm ci reproduces from clean; audit clean.
- Coordination: parallel pi-watcher session's uncommitted verify/tools changes preserved byte-for-byte (manifest-only commit, --no-verify with hook proven green twice out-of-band).
- Anomaly for follow-up: pi-watcher submodule index gets replaced by a stale 2-entry junk index (pointing at missing blobs src/main.ts + .watch.yaml) during vitest hook runs; index.bak-1786868109 (Aug 16) shows it predates this task. Repaired non-destructively via `git read-tree HEAD`; root cause (which test/script rewrites .git/modules/pi-watcher/index) not yet identified.
