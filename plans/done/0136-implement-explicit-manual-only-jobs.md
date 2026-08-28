---
id: TASK-0136
title: Implement explicit manual-only jobs
status: todo
depends_on: [TASK-0135]
priority: high
tags: [rust, config, jobs, control-socket, schema, tdd]
---

# Implement explicit manual-only jobs

## Problem

Users need declared finite jobs that remain excluded from filesystem and initialization plans while still being selectable through existing local and control run APIs.

## Context

Implement the contract from TASK-0135. Reuse existing target selection, execution, generation identity, output retention, cancellation, and control APIs. This is a configuration/routing capability, not a second process executor.

## Acceptance criteria

- [ ] Write failing parser, validation, matching, target-selection, and reload-revision tests before production changes.
- [ ] Parse the approved explicit manual-only shape in preferred `jobs:` configuration.
- [ ] Reject ambiguous combinations identified by TASK-0135 with one actionable configuration error.
- [ ] Exclude manual-only jobs from root/per-job path matching and initialization plans.
- [ ] Keep manual-only jobs selectable through `fzz run TARGET` and `fzz ctl run TARGET`, including existing exact-name, tag, substring, and ambiguity behavior.
- [ ] Make `fzz list` expose manual trigger mode and `fzz explain PATH` exclude the job from path-selected plans without implying it is unavailable.
- [ ] Add the option to canonical schema/catalog/help and generated configuration surfaces without hand-maintained drift.
- [ ] Include effective trigger mode in semantic configuration revision identity and preserve frozen behavior for already-running generations.
- [ ] Preserve byte-for-behavior compatibility for configs without the new explicit shape and for legacy task configurations.
- [ ] Keep control protocol method names and payloads unchanged unless the contract proves an additive capability field is required; coordinate any wire change with `pi-watcher`.
- [ ] Do not add arbitrary command execution, provider fields, result schemas, per-run environment input, timeout behavior, or service lifecycle changes.

## Verification focus

Use focused Rust tests during TDD. The black-box user workflow and documentation belong to TASK-0137.
