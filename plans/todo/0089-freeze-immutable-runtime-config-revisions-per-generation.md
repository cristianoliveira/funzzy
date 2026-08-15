---
id: TASK-0089
title: Freeze immutable runtime config revisions per generation
status: todo
depends_on: [TASK-0088, TASK-0025, TASK-0052]
priority: high
tags: [rust, config, revision, executor, identity, tdd]
---

# Freeze immutable runtime config revisions per generation

## Problem
In-process reload needs one validated immutable runtime snapshot so active work cannot observe a partial mixture of old jobs and new policy while later generations use new configuration.

## Context

Introduce domain `RuntimeConfig`/`ConfigRevision`; composition root owns building it. Generations hold `Arc` snapshot rather than reading mutable global config.

## Acceptance criteria

- [ ] Tests first prove deterministic semantic hash/revision behavior for identical formatting-only save, job/topology/root/policy changes, and secrets-safe metadata.
- [ ] Candidate parser builds complete immutable runtime config off to side with jobs, matching, roots, concurrency, debounce, backend, gitignore, hooks/policies, services, control options, and signatures.
- [ ] Successful semantic change increments monotonic revision; no-op/comment-only rewrite reports no-op without generation or subsystem churn.
- [ ] Event batch captures one revision before plan creation and generation/snapshot/outcome retain same revision through terminal state.
- [ ] Active/queued generation semantics at reload boundary are explicit; no plan combines jobs/signature from different revisions.
- [ ] Duration signature/history keys derive frozen effective config and formatting-only reload does not invalidate history.
- [ ] Invalid candidate cannot publish revision or mutate live objects before fatal shutdown path owns cleanup.
- [ ] Public diagnostics expose revision number and non-secret hash; declared environment values remain secret-safe.
- [ ] Existing local run remains finite immutable config and does not gain file-reload behavior.

## Notes
