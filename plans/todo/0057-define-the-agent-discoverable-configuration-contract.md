---
id: TASK-0057
title: Define the agent-discoverable configuration contract
status: todo
depends_on: [TASK-0042]
priority: high
tags: [design, axi, config, schema, agents, determinism]
---

# Define the agent-discoverable configuration contract

## Problem
Agents can execute Funzzy but cannot discover current `.watch.yaml` fields, constraints, defaults, compatibility forms, or safe next commands without reading repository/web documentation.

## Context

Primary discovery lives in installed CLI, not repository docs or Pi-specific prompt. Prefer standard JSON Schema for interoperable structure, bounded section queries for token cost, and generated runnable YAML examples. `fzz check` remains semantic validation command from TASK-0033.

## Acceptance criteria

- [ ] Contract defines agent decision loop: discover schema → request relevant section/example → write config → check → list/explain → run/watch.
- [ ] Locks command grammar for `fzz config schema [--section SECTION]` and `fzz config example PROFILE`; commands are non-interactive and side-effect-free.
- [ ] Defines supported schema sections (`on`, `task`, `matching`, `execution`, `parallel`, `control`) and example profiles (`minimal`, `parallel`, `agent`).
- [ ] JSON Schema is canonical interoperability output; compact text/TOON may be additive but cannot replace valid JSON Schema.
- [ ] Schema identifies version, field type, required/default status, enum/range, mutual constraints, deprecation, examples, and semantic checks delegated to `fzz check`.
- [ ] Current grouped config is recommended; legacy root task list remains documented as accepted compatibility input but is not emitted.
- [ ] Output bounds, stable ordering, stdout/stderr separation, exit codes, unknown section/profile recovery hints, and config-free operation are explicit.
- [ ] One declarative config-spec source or enforced parity tests prevent schema/examples/help/parser key drift.
- [ ] Security boundary excludes environment values, resolved secrets, filesystem contents, and running watcher state.
- [ ] Contract defines generated agent guide/skill as optional secondary artifact from same spec, never independently maintained truth.

## Notes

Coordinate with TASK-0033 (`fzz check`) and TASK-0048 structured output without blocking standard JSON Schema discovery on optional TOON rendering.

