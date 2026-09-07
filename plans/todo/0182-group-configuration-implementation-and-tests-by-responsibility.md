---
id: TASK-0182
title: Group configuration implementation and tests by responsibility
status: todo
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

## Non-goals

No repository-wide style sweep, bulk deletion of contract comments, semantic renaming, or domain/application/infra directory migration.
