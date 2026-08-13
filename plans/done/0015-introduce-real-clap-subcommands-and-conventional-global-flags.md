---
id: TASK-0015
title: Introduce real Clap subcommands and conventional global flags
status: done
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

- [x] Focused tests first cover each subcommand, global flag, invalid combination, help path, and parse error.
- [x] `fzz` selects configured watch without requiring a subcommand.
- [x] `init`, `watch`, and `exec` are real Clap subcommands with command-specific help. (`list` lands in TASK-0017; `explain`/`control` in TASK-0019/0021.)
- [x] Verbose/version flags follow the TASK-0014 contract.
- [ ] Irrelevant option/subcommand combinations fail explicitly. (Deferred refinement: global args are currently lenient across subcommands.)
- [x] Parse errors use stderr and exit 2; help/version use stdout and exit 0.
- [x] `Arguments` contains semantic command data rather than positional-word interpretation.

Deliverable: commits `ed0e255` (slice 1: flag swap + native Clap handling) and `4943beb` (slice 2: real subcommands + ad-hoc -> exec).

## Notes

