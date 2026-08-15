---
id: TASK-0082
title: Publish exact output references in correlated snapshots
status: todo
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

- [ ] Failed task outcome/failure evidence includes structured `outputRef`; whole-generation terminal snapshot can include safe aggregate reference when retained output exists.
- [ ] Reference instance, generation, task ID, and availability are frozen/correlated to same snapshot source used by await/subscription/status.
- [ ] Status, atomic await, subscribe notification, verify result, and structured CLI render same reference without divergent reconstruction.
- [ ] Capability exposes output schema version/request variants/byte limit and fixtures prove old clients ignore additive fields.
- [ ] Human retrieval command uses shell-safe exact identity and bounded option generated from reference; tags/spaces/quotes cannot corrupt command.
- [ ] No reference is emitted before relevant output exists or after known eviction; retrieval still handles race with typed eviction error.
- [ ] Multiple failures each retain own reference while compact status chooses deterministic primary reference and declares additional count.
- [ ] Success with no useful output does not encourage retrieval; cancellation/supersession references follow policy explicitly.
- [ ] JSON, TOON, and text outputs preserve identity and stay bounded.

## Notes
