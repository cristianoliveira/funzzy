---
id: TASK-0037
title: Add native watcher polling fallback
status: todo
depends_on: [TASK-0031]
priority: normal
tags: [rust, watcher, polling, portability, reliability, tdd]
---

# Add native watcher polling fallback

## Problem
Native filesystem backends may fail or behave poorly in containers, network filesystems, WSL, and unusual platform environments, leaving no reliable fallback.

## Context

Add explicit backend policy: native, poll, or auto fallback. Both feed same event batching/matching path.

## Acceptance criteria

- [ ] Config/CLI contract defines backend selection and polling interval with validated duration.
- [ ] Auto tries native first and emits one actionable warning before deterministic polling fallback.
- [ ] Forced native fails clearly instead of silently changing semantics.
- [ ] Poll backend detects create/modify/remove/rename-equivalent changes needed by matching.
- [ ] Poll and native events normalize into same `EventBatch` contract.
- [ ] Fake filesystem/clock tests avoid sleeps; black-box test proves fallback and shutdown.
- [ ] CPU/scalability tradeoffs and container/network filesystem use cases are documented.

## Notes

