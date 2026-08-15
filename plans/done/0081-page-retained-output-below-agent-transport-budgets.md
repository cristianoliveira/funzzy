---
id: TASK-0081
title: Page retained output below agent transport budgets
status: done
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

Contract: `docs/OUTPUT-EVIDENCE-CONTRACT.md` §4 (budget) + §5 (paging).

- [x] Tests first cover zero/small/large output, multiple tasks/streams, UTF-8/non-UTF-8 replacement, long lines, truncation, exact boundary, and envelope overhead. (`src/output.rs` page tests: first-chunk, resume, ordering, UTF-8 boundary, pathological escaping ≤ budget, unknown generation/task, stale/tampered cursor, canonical resolution; `src/control.rs` page-mode tests.)
- [x] Request supports deterministic page/maxBytes contract below negotiated limit; ordering is stable by task identity then stream and cursor cannot skip/duplicate bytes. (`retrieve_page`: task ID sort → stdout → stderr → byte order; measured serialized budget; resume test proves no skip/duplicate.)
- [x] Default request returns bounded failure-focused tail; whole-generation retrieval shares budget rather than multiplying limit per task/stream. (tail remains default; page mode shares one budget across the whole generation.)
- [x] Unsafe unpaged `full` is removed from preferred agent contract or deterministically translated to first bounded page with continuation, never > transport budget. (`full` → `retrieve_page` with default budget; test asserts serialized ≤ page budget.)
- [x] `tail` versus paging/full variants are structurally exclusive and Rust CLI rejects conflicts before socket call. (`--page` conflicts with `--tail`/`--full` via clap; `--page-size`/`--cursor` require `--page`; exit 2 tests; server also rejects `mode` conflicts with `-32013`.)
- [x] Response reports next cursor/null, returned bytes, remaining/truncated state, retained/observed bytes, and eviction between pages. (`nextCursor`, `returnedBytes`, `truncated` additive on `RetrievedOutput`; per-stream retained/observed already present; eviction → `-32010` typed.)
- [x] Cursor is instance/generation/task/stream scoped, opaque or validated, and stale/tampered cursor gets typed error. (cursor `<gen>|<plan>|<stream>|<offset>` validated against plan; generation mismatch, out-of-range, wrong stream, offset beyond retained → `-32013 invalid_options`.)
- [x] Capability `maxResponseBytes` reflects serialized RPC envelope guarantee and remains conservative across JSON/TOON. (`outputSchemaVersion: 2`, `outputModes: [tail, page]`, `outputPageSizeMax: 32768`, `outputMaxBytesEffective: 24576` < `maxResponseBytes: 65536`; pi-watcher fixture updated, decoder tests green.)
- [x] Memory retention remains globally bounded; pagination never copies unbounded buffers or holds registry lock during socket writes. (retention budget unchanged; page content bounded by budget; lock held only to slice retained bytes, serialization happens on the borrowed slices.)

## Notes
