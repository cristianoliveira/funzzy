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

## Hooks layout increment evidence

- Source locations and move proof: `.tmp/reports/04-09-26/task-0182-hooks-layout.md`.
- Commit `4a3eb5e` moves the complete hooks production/test cluster next to the backend/gitignore cluster. `config::hooks_tests::*` paths and all public symbols remain unchanged; no semantics or comments changed.
- Config tests 127, architecture guard 8, and fresh watcher gen215 pass. Remaining clusters: output policy, service/readiness, V2, catalog/manual, and timeout.

## Output layout increment evidence

- Source locations and move proof: `.tmp/reports/04-09-26/task-0182-output-layout.md`.
- Commit `230205d` moves `output_policy_tests` and `output_policy_from_yaml` adjacent to hooks/backend policy groups. Test path, assertions, comments, public signature/visibility, parser output, and errors are unchanged; service/readiness was not moved.
- Config tests 127, architecture guard 8, and fresh watcher gen216 pass. Remaining clusters: service/readiness, V2, catalog/manual, and timeout.

## Service/readiness layout increment evidence

- Source locations and move proof: `.tmp/reports/04-09-26/task-0182-service-layout.md`.
- Commit `406164f` moves only `service_tests` beside its existing `readiness_from_yaml` implementation; no service/readiness production code, test path, assertion, comment, visibility, or behavior changed.
- Config tests 127, architecture guard 8, and fresh watcher gen217 pass. Remaining clusters: V2, catalog/manual, and timeout.

## V2 layout increment evidence

- Source locations and move proof: `.tmp/reports/04-09-26/task-0182-v2-layout.md`.
- Commit `e34acdf` moves `v2_section_tests` directly after `validate_v2_sections`; test path, assertions, comments, visibility, parser results, and contract/error checks are unchanged.
- Config tests 127, architecture guard 8, AST map parsing, and fresh watcher gen218 pass. Remaining clusters: catalog/allowlist, manual trigger, and timeout.

## Catalog/allowlist layout increment evidence

- Source locations and move proof: `.tmp/reports/04-09-26/task-0182-catalog-layout.md`.
- Commit `59de8b0` moves `catalog_allowlist_tests` directly after V2 validation tests; test path, assertions, comments, visibility, catalog contracts, parser results, and errors are unchanged.
- Config tests 127, architecture guard 8, and fresh watcher gen219 pass. Remaining clusters: manual trigger and timeout.

## Manual-trigger layout increment evidence

- Source locations and move proof: `.tmp/reports/04-09-26/task-0182-manual-layout.md`.
- Commit `71b8f54` moves `manual_trigger_tests` directly after `rule_from_with_common`; test path, assertions, comments, helper/signature, visibility, compatibility checks, and error contracts are unchanged.
- Config tests 127, architecture guard 8, and fresh watcher gen220 pass. Remaining cluster: timeout.
