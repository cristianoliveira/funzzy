---
id: TASK-0014
title: Define the V2 CLI contract
status: done
depends_on: []
priority: high
tags: [rust, cli, design]
---

# Define the V2 CLI contract

## Problem
The current help presents pseudo-subcommands and hides ambiguous positional, target-list, and flag behavior. We need one explicit contract before changing parser or runtime behavior.

## Context

Use `.tmp/reports/13-04-26/cli-interface-review.md` and `.tmp/reports/13-04-26/similar-cli-inspiration.md` as research input. This is intentionally a V2 contract: do not add deprecated parser paths during the active refactor.

## Acceptance criteria

- [x] Command tree defines zero-argument watch, `watch`, `list`, `init`, `exec`, and control-socket client behavior.
- [x] Ownership and valid scope of every option is explicit.
- [x] Child argv boundary, stdin meaning, target matching, exit codes, stdout/stderr, and environment precedence are specified.
- [x] `-v/--verbose` and `-V/--version` conventions are decided.
- [x] Verbose diagnostic vocabulary, event/run correlation identity, effective rule origin, feedback-loop heuristic, command exposure, deterministic fields, and stdout/log-file behavior are decided.
- [x] Busy-run vocabulary and defaults are decided.
- [x] Control client command names, socket-path precedence, wait semantics, timeout behavior, and exit codes are decided.
- [x] Synthetic `emit PATH` contract defines normalization, nonexistent/deleted paths, unmatched response, run identity, and exact equivalence with native change routing.
- [x] Contract includes happy and unhappy black-box test matrix for both `funzzy` and `fzz`.
- [x] Compatibility breaks and migration guidance are listed before implementation starts.

Deliverable: `docs/CLI-V2-CONTRACT.md`.

## Notes

