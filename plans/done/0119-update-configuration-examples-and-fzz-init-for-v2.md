---
id: TASK-0119
title: Update configuration examples and fzz init for V2
status: done
depends_on: [TASK-0117]
priority: high
tags: [rust, cli, init, examples, config, v2, tdd]
---

# Update configuration examples and fzz init for V2

## Problem
Generated examples and `fzz init` are primary configuration entry points and must emit only the new V2 structure without drifting from schema or parser.

## Context

`fzz init --template PROFILE` and `fzz config example PROFILE` already share renderers and byte-parity tests. Preserve that architecture.

## Acceptance criteria

- [x] Write failing generator tests first asserting every profile uses only canonical V2 `on`, `execution`, `hooks`, and `jobs` ownership, with no version property.
- [x] Update the canonical option catalog and comprehensive renderer so `fzz init` documents every V2 property under its owning section.
- [x] Update `minimal`, `parallel`, and `agent` examples; add `execution` or `hooks` only when the profile teaches them rather than emitting empty sections.
- [x] Ensure the agent example keeps the control socket under `on` and the parallel example teaches concurrency under `execution`.
- [x] Include a runnable hook example in the comprehensive template with clear lifecycle wording.
- [x] Keep `fzz init` create-only behavior, template selection, deterministic bytes, size/readability budget, and existing-file refusal unchanged.
- [x] Keep each `fzz init --template P` byte-identical to `fzz config example P`.
- [x] Prove every generated artifact passes production V2 parsing, `fzz check`, and at least one representative run where appropriate.
- [x] Prove generated artifacts contain no V1-only placement such as `on.concurrency`, `on.output`, or `on.success`.
- [x] Update CLI help around profiles only where ownership examples changed.

## Outcome

Generated profiles emit only preferred V2 section ownership. The comprehensive template actively demonstrates `execution.concurrency` and a finite `hooks.success` lifecycle reaction.

## Notes

Generated outputs must teach the preferred format; do not emit compatibility syntax.
