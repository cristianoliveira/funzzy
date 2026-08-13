---
id: TASK-0014
title: Define the V2 CLI contract
status: todo
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

- [ ] Command tree defines zero-argument watch, `watch`, `list`, `init`, `exec`, and control-socket client behavior.
- [ ] Ownership and valid scope of every option is explicit.
- [ ] Child argv boundary, stdin meaning, target matching, exit codes, stdout/stderr, and environment precedence are specified.
- [ ] `-v/--verbose` and `-V/--version` conventions are decided.
- [ ] Busy-run vocabulary and defaults are decided.
- [ ] Control client command names, socket-path precedence, wait semantics, timeout behavior, and exit codes are decided.
- [ ] Synthetic `emit PATH` contract defines normalization, nonexistent/deleted paths, unmatched response, run identity, and exact equivalence with native change routing.
- [ ] Contract includes happy and unhappy black-box test matrix for both `funzzy` and `fzz`.
- [ ] Compatibility breaks and migration guidance are listed before implementation starts.

## Notes

