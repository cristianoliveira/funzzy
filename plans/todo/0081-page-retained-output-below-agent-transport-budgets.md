---
id: TASK-0081
title: Page retained output below agent transport budgets
status: todo
depends_on: [TASK-0079, TASK-0080]
priority: high
tags: [rust, output, pagination, bounds, axi, tdd]
---

# Page retained output below agent transport budgets

## Problem
Whole-generation and full retrieval can exceed Pi 64KB despite bounded server retention, while tool input permits full plus tail and lacks deterministic pagination.

## Context

Pi transport currently rejects responses above 64KB. Reserve envelope/encoding margin rather than setting server payload limit equal to transport maximum.

## Acceptance criteria

- [ ] Tests first cover zero/small/large output, multiple tasks/streams, UTF-8/non-UTF-8 replacement, long lines, truncation, exact boundary, and envelope overhead.
- [ ] Request supports deterministic page/maxBytes contract below negotiated limit; ordering is stable by task identity then stream and cursor cannot skip/duplicate bytes.
- [ ] Default request returns bounded failure-focused tail; whole-generation retrieval shares budget rather than multiplying limit per task/stream.
- [ ] Unsafe unpaged `full` is removed from preferred agent contract or deterministically translated to first bounded page with continuation, never > transport budget.
- [ ] `tail` versus paging/full variants are structurally exclusive and Rust CLI rejects conflicts before socket call.
- [ ] Response reports next cursor/null, returned bytes, remaining/truncated state, retained/observed bytes, and eviction between pages.
- [ ] Cursor is instance/generation/task/stream scoped, opaque or validated, and stale/tampered cursor gets typed error.
- [ ] Capability `maxResponseBytes` reflects serialized RPC envelope guarantee and remains conservative across JSON/TOON.
- [ ] Memory retention remains globally bounded; pagination never copies unbounded buffers or holds registry lock during socket writes.

## Notes
