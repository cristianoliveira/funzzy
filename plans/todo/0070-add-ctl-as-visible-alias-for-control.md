---
id: TASK-0070
title: Add ctl as visible alias for control
status: todo
depends_on: [TASK-0015, TASK-0021]
priority: high
tags: [rust, cli, clap, control-socket, axi, tdd]
---

# Add ctl as visible alias for control

## Problem
Repeated control-socket operations are verbose for humans and agents, while a conventional ctl alias can shorten commands without replacing clear canonical control vocabulary or duplicating dispatch logic.

## Context

Use Clap `visible_alias("ctl")` on canonical `control` command. Keep one command definition and one `ControlAction` dispatch path. `control` remains canonical documentation, diagnostics, and protocol vocabulary.

## Acceptance criteria

- [ ] Parser tests first prove `control` and `ctl` produce identical action/arguments for capabilities, status, list, run, emit, await, cancel, and output.
- [ ] `fzz ctl --help` and `fzz control --help` expose same nested command tree and exit 0.
- [ ] Top-level help visibly advertises `ctl` as alias without rendering duplicate command entry or changing canonical usage unexpectedly.
- [ ] Black-box parity tests cover both `fzz` and `funzzy`, representative success, usage error, socket failure, structured output, and exit codes.
- [ ] Global and control-local options retain same scope/precedence through alias, including config/socket and wait/timeout arguments.
- [ ] Unknown nested operation fails with exit 2 and same actionable alternatives for either spelling.
- [ ] No aliases are added for nested operations; protocol JSON-RPC method names remain unchanged.
- [ ] Shell completion/help generation includes alias through Clap rather than handwritten duplication.
- [ ] V2 migration/command docs identify `control` as canonical and mention `ctl` once as convenience; examples may use either consistently within one page.

## Notes

This is additive CLI convenience, not a protocol or compatibility break.

