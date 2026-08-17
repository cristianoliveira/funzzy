---
id: TASK-0106
title: Update pi-watcher Node and Pi dependencies
status: todo
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

- [ ] Capture `npm outdated`, `npm audit`, package tree, `npm pack --dry-run`, and `make all` baseline before manifest changes.
- [ ] Update `@earendil-works/pi-ai` and `@earendil-works/pi-coding-agent` dev pins together to same tested Pi release; verify public extension APIs and TUI/tool registration compile.
- [ ] Update TypeBox and YAML within compatible stable lines and confirm runtime decoders still accept additive Funzzy protocol fields and reject malformed payloads.
- [ ] Evaluate TypeScript 7 separately against Node 24, types/node, ESLint/typescript-eslint, Vitest/coverage, Pi packages, and current tsconfig; upgrade only as coherent toolchain or document deferral.
- [ ] Refresh remaining lint/format/test dependencies only to mutually supported versions; avoid `--force` or legacy peer resolution.
- [ ] Keep runtime `dependencies`, Pi `peerDependencies`, and test/build `devDependencies` correctly classified; no Pi runtime package is accidentally bundled.
- [ ] `npm ci` reproduces package-lock from clean checkout and `npm audit --audit-level=high` remains green.
- [ ] `make quick`, `make all`, real-socket e2e, and `npm pack --dry-run` pass; package file list and tool names remain unchanged unless deliberate.
- [ ] Commit extension changes in pi-watcher repository, then update root submodule gitlink explicitly with compatibility evidence.

## Notes

Current pi-watcher worktree is ahead of origin; reconcile ownership/current work before changing it and never overwrite another agent’s changes.

