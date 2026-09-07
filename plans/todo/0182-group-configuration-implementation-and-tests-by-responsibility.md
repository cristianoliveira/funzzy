---
id: TASK-0182
title: Group configuration implementation and tests by responsibility
status: doing
depends_on: [TASK-0178]
priority: normal
tags: [rust, config, readability]
---

# Group configuration implementation and tests by responsibility

## Problem
Configuration production definitions resume after large test modules. Historical comments and scattered layout make the owning implementation hard to locate.

## Context

At review baseline, tests start around `src/config.rs:1377`, while production hook types/parsers resume around line 3038. This follow-up waits for TASK-0178 to establish parser ownership, avoiding a cosmetic move that would immediately be redone.

## Scope and approach

- Group production definitions by the responsibilities established by the shared-document refactor.
- Place each test suite beside its owning private submodule or in a dedicated private test module. Keep all production sections discoverable without searching past unrelated test bodies.
- Shorten repetitive qualification with explicit local imports where it clarifies the code.
- Lead comments with current behavior or rationale. Preserve useful contract references and ordering explanations; remove only redundant historical narration.
- Keep this as a mechanical layout/documentation change, separate from behavior fixes.

## Acceptance criteria

- [ ] Each config responsibility has a clear source location and nearby test ownership.
- [ ] No production implementation is hidden between unrelated test suites.
- [ ] Existing test cases and assertions are preserved; any renamed test paths are mapped in verification notes.
- [ ] Comments still explain legacy compatibility, default/error precedence, and non-obvious invariants.
- [ ] No public type/function path, visibility contract, parser result, or error message changes.
- [ ] A reader can locate document decoding, validation delegation, file loading, and YAML presentation from the module outline.

## Verification

Review a move-aware diff, compare test inventory before/after, run focused config tests and architecture guards, then the fresh watcher final gate. File-size reduction alone is not evidence of improved behavior or coverage.

## Likely files

`src/config.rs` and private configuration/test submodules established by TASK-0178.

## First layout increment evidence

- Inventory and before/after source locations: `.tmp/reports/04-09-26/task-0182-backend-layout.md`.
- Commit `c6f0743` moves backend/watch-backend and gitignore implementations together with their existing test modules from after `jobs_tests` to immediately before the characterization tests. Test module paths, assertions, comments, visibility, and parser behavior are unchanged.
- Focused config tests: 127 passed; architecture guard `domain_boundaries`: 8 passed; fresh watcher gen214 passed.
- Remaining layout gaps are explicitly tracked: hooks, output policy, service/readiness, V2 sections, catalog/manual, and timeout groups still follow broad test modules. No broad rewrite or semantic move was attempted.
