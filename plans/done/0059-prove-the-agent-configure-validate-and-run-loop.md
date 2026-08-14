---
id: TASK-0059
title: Prove the agent configure validate and run loop
status: done
depends_on: [TASK-0058, TASK-0033]
priority: high
tags: [integration-tests, axi, config, schema, agents, compatibility]
---

# Prove the agent configure validate and run loop

## Problem
Schema and examples can drift from the real parser unless black-box tests prove an agent can discover fields, create configuration, validate it, inspect targets, and execute one target without external documentation.

## Context

Use black-box subprocesses and isolated workspace. Test decisions/output contracts, not one specific LLM or prompt.

## Acceptance criteria

- [ ] Agent-style E2E starts without config, discovers relevant schema section, requests example, writes it, and runs `fzz check` successfully.
- [ ] Same E2E lists targets, explains matching and ignored paths, and executes exact finite target without starting watcher/socket accidentally.
- [ ] Mutating one structural field and one semantic field yields deterministic path-specific diagnostics plus recovery command.
- [ ] Full schema and all examples remain valid, deterministic, bounded, secret-free, and synchronized with production parser across CI.
- [ ] Legacy config is accepted/checkable but discovery recommends current grouped form and migration command.
- [ ] Unsupported installed version/schema mismatch is explicit and does not silently guess fields.
- [ ] No command prompts, reaches network, executes task during discovery/check, or writes outside explicit redirected example output.
- [ ] Representative command round trips and output byte/token sizes are recorded; section query materially reduces full-schema cost.
- [ ] README agent section documents only bootstrap commands and links installed self-description as source of truth.
- [ ] Focused tests and watcher verification pass with unchanged worktree fingerprint.

## Notes

