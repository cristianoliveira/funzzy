---
id: TASK-0015
title: Introduce real Clap subcommands and conventional global flags
status: todo
depends_on: [TASK-0014]
priority: high
tags: [rust, cli, clap, tdd]
---

# Introduce real Clap subcommands and conventional global flags

## Problem
Funzzy cannot provide accurate command-specific help or reject irrelevant option combinations while init and watch remain positional words.

## Context

Let Clap model the command hierarchy directly. Keep `main.rs` as composition root and expose semantic actions from `arguments.rs`.

## Acceptance criteria

- [ ] Focused tests first cover each subcommand, global flag, invalid combination, help path, and parse error.
- [ ] `fzz` selects configured watch without requiring a subcommand.
- [ ] `init`, `watch`, `list`, and `exec` are real Clap subcommands with command-specific help.
- [ ] Verbose/version flags follow the TASK-0014 contract.
- [ ] Irrelevant option/subcommand combinations fail explicitly.
- [ ] Parse errors use stderr and the contract's usage exit code; help/version use stdout and exit 0.
- [ ] `Arguments` contains semantic command data rather than positional-word interpretation.

## Notes

