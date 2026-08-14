---
id: TASK-0077
title: Separate configured jobs from runtime task identities
status: done
depends_on: [TASK-0075, TASK-0076, TASK-0043]
priority: high
tags: [rust, domain, naming, jobs, tasks, protocol, refactor]
---

# Separate configured jobs from runtime task identities

## Problem
Renaming one YAML key without defining domain boundaries would leave CLI, diagnostics, control payloads, plans, and documentation using job and task inconsistently.

## Context

Apply ubiquitous language at boundaries without a risky blind symbol rename: `Job` is configured definition/plan entry; `Task` is its execution/outcome in one generation; `Command` remains sequential child process invocation.

## Acceptance criteria

- [ ] Domain glossary and public naming table classify config, planning, executor, output, control, history, CLI, and docs terms before refactor.
- [ ] Config/parser/list/init user messages say jobs where referring to configured entries.
- [ ] Run plans preserve stable configured job name/position and derive runtime task identity/group occurrence without ambiguity.
- [ ] Control target/list vocabulary is documented; existing JSON keys remain additive-compatible and are not duplicated merely for wording.
- [ ] Error messages distinguish invalid job configuration from failed runtime task/command.
- [ ] Duration execution signature remains semantic: tasks→jobs spelling migration alone does not invalidate history, while actual job content/topology does.
- [ ] No broad search/replace changes unrelated test descriptions, protocol fixtures, or historical contracts.
- [ ] LSP/AST usage review proves public/internal rename impact and no dead duplicate Task/Job model remains.
- [ ] CLI help, schema, examples, and migration use one glossary consistently.

## Notes

