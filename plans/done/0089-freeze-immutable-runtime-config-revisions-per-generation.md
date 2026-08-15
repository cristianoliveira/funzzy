---
id: TASK-0089
title: Freeze immutable runtime config revisions per generation
status: done
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

Domain: `src/config_revision.rs` (`RuntimeConfig`, `ConfigRevision`, `RevisionTracker`, `semantic_hash`); revision rides `RunMetadata` → `Event::Started` → `ControlState` → `CorrelatedSnapshot`.

- [x] Tests first prove deterministic semantic hash/revision behavior for identical formatting-only save, job/topology/root/policy changes, and secrets-safe metadata. (11 unit tests: identical configs hash equal; formatting-only rewrite hashes equal and is `NoOp`; semantic change increments monotonic revision; policy/backend/debounce/gitignore/ignore/service changes are semantic; env VALUES never enter the hash, keys are semantic; hash stable across calls.)
- [x] Candidate parser builds complete immutable runtime config off to side with jobs, matching, roots, concurrency, debounce, backend, gitignore, hooks/policies, services, control options, and signatures. (`RuntimeConfig::capture` + `semantic_hash` encodes rules, patterns, commands (hashed), cwd, env keys, concurrency, debounce, backend, gitignore, hooks; `plan()` derives the frozen plan.)
- [x] Successful semantic change increments monotonic revision; no-op/comment-only rewrite reports no-op without generation or subsystem churn. (`RevisionTracker::observe` returns `New`/`NoOp`; unit-tested.)
- [x] Event batch captures one revision before plan creation and generation/snapshot/outcome retain same revision through terminal state. (`RunMetadata.revision/revision_hash` captured before plan; `ControlState` sets on Started and retains through Finished/Cancelled — unit test `finished_records_terminal_state...` asserts retention; snapshot exposes it.)
- [x] Active/queued generation semantics at reload boundary are explicit; no plan combines jobs/signature from different revisions. (Each generation carries its frozen revision number + hash; plans derive from one `RuntimeConfig::plan()`.)
- [x] Duration signature/history keys derive frozen effective config and formatting-only reload does not invalidate history. (`history_tests`: formatting-only rewrite keeps identical `execution_signature`; semantic change invalidates it.)
- [x] Invalid candidate cannot publish revision or mutate live objects before fatal shutdown path owns cleanup. (Tracker publishes only via `observe` after a fully-built candidate; composition root validates first; fatal cleanup is TASK-0090 scope.)
- [x] Public diagnostics expose revision number and non-secret hash; declared environment values remain secret-safe. (Snapshot `revision` + `revisionHash` on status/await/subscribe; hash never contains env values — unit-tested.)
- [x] Existing local run remains finite immutable config and does not gain file-reload behavior. (`fzz run` uses `RunMetadata::new` → revision None; no reload coupling — verified.)

## Notes
