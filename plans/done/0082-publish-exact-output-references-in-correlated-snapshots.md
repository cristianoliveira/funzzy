---
id: TASK-0082
title: Publish exact output references in correlated snapshots
status: done
depends_on: [TASK-0079, TASK-0080, TASK-0081, TASK-0044, TASK-0050]
priority: high
tags: [rust, snapshots, output, capabilities, agents, freshness]
---

# Publish exact output references in correlated snapshots

## Problem
Agents currently reconstruct task names from human summaries; snapshots and failure evidence need copy-safe instance/generation/task references and negotiated output schema/limits.

## Context

Reference originates where server knows exact identity: terminal snapshot/failure evidence. Human `next:` text is projection of structured data, not source of truth.

## Acceptance criteria

Contract: `docs/OUTPUT-EVIDENCE-CONTRACT.md` §1 (outputRef) + §4 (capabilities) + §5 (paging).

- [x] Failed task outcome/failure evidence includes structured `outputRef`; whole-generation terminal snapshot can include safe aggregate reference when retained output exists. (`FailureEvidence.output_ref`: instanceToken + generation + exact task + mode/tail/maxBytes + shell-safe retrieve; `CorrelatedSnapshot.failure_evidence` on subscribe.)
- [x] Reference instance, generation, task ID, and availability are frozen/correlated to same snapshot source used by await/subscription/status. (single `failure_evidence` builder reads the same registry + instance token at the boundary; instance threaded into status, await, and broker.)
- [x] Status, atomic await, subscribe notification, verify result, and structured CLI render same reference without divergent reconstruction. (status_result, awaiting build, SnapshotBroker.build all emit `FailureEvidence` with `output_ref`; CLI renders outputRef + additionalFailedTasks; subscribe E2E test proves the notification carries it.)
- [x] Capability exposes output schema version/request variants/byte limit and fixtures prove old clients ignore additive fields. (`outputSchemaVersion: 2`, `outputModes`, `outputPageSizeMax`, `outputMaxBytesEffective` from TASK-0081; pi-watcher decoders read known keys only — 441 tests green.)
- [x] Human retrieval command uses shell-safe exact identity and bounded option generated from reference; tags/spaces/quotes cannot corrupt command. (`shell_quote` POSIX `'\''` idiom; test with task ID containing tags, spaces, and single quotes.)
- [x] No reference is emitted before relevant output exists or after known eviction; retrieval still handles race with typed eviction error. (`output_ref` only when `total_retained > 0`; eviction → typed `-32010`.)
- [x] Multiple failures each retain own reference while compact status chooses deterministic primary reference and declares additional count. (`additional_failed_tasks` counts beyond the first retained task; primary = first retained; test with two failed tasks.)
- [x] Success with no useful output does not encourage retrieval. (empty capture → no output_ref, no retrieve hint.)
- [x] JSON, TOON, and text outputs preserve identity and stay bounded. (`await_document` carries outputRef in structured formats; human renderer shows instance/task/retrieve; bounded by design.)

## Notes
