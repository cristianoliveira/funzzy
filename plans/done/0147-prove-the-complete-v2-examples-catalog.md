---
id: TASK-0147
title: Prove the complete V2 examples catalog
status: todo
depends_on: [TASK-0146]
priority: normal
tags: [examples, integration-tests, docs, config, v2, reliability]
---

# Prove the complete V2 examples catalog

## Problem

Without a recursive validation and behavior gate, YAML extension gaps, intentionally invalid fixtures, stale paths, or semantic drift can leave shipped examples misleading or broken.

## Acceptance criteria

- [ ] Convert the 3 intentionally invalid fixtures to preferred V2-invalid shapes while preserving and documenting each intended failure reason.
- [ ] Replace the top-level `.yml`-only check with deterministic recursive discovery of both `.yml` and `.yaml`; every non-invalid config must pass production `fzz check`.
- [ ] Assert each invalid fixture fails for its named expected reason and `fzz migrate` leaves it byte-identical.
- [ ] Assert all valid examples contain canonical V2 root structure and no `tasks:`/root-list teaching surface; all are migration no-ops.
- [ ] Run focused behavior tests covering every matrix row from TASK-0144, including nested flattening and `.yaml` long-running fixtures.
- [ ] Update `examples/README.md` with explicit V2 watch/run/check commands, catalog purpose, and separation of runnable vs invalid fixtures.
- [ ] Verify no stale old filename references or dead documentation links remain.
- [ ] Run formatter/lint/unit/integration gates appropriate to config, migration, filesystem watch, and docs changes; record evidence in `.tmp/reports/`.

## Test constraints

Reuse established per-test example fixture copies and bounded readiness polling. Never add shared mutable example state or fixed-sleep assertions.
