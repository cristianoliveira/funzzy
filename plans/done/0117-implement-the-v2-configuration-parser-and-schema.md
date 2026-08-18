---
id: TASK-0117
title: Implement the V2 configuration parser and schema
status: done
depends_on: [TASK-0116]
priority: high
tags: [rust, config, schema, validation, tdd]
---

# Implement the V2 configuration parser and schema

## Problem
Runtime parsing and agent-discoverable schema must enforce the same coherent V2 shape before any generator or migration can safely emit it.

## Context

The option catalog is the current shared owner for parser allowlists, schema fields, and comprehensive init content. Extend that ownership rather than creating parallel section lists.

## Acceptance criteria

- [x] Write failing parser tests first for canonical V2, each new root section, existing V1 task-list compatibility, unknown keys, wrong types, malformed hooks, and the contract decision for previous grouped V2 field placements.
- [x] Extend the catalog ownership model from `Root | On | Job` to include `Execution` and `Hooks`; keep one property identity and ordering source.
- [x] Parse V2 `on`, `execution`, `hooks`, and ordered `jobs` into existing runtime policy types without changing job ordering, matching, hook lifecycle, socket, or execution behavior.
- [x] Preserve existing V1 root task-list compatibility in ordinary runtime paths; do not broaden or redefine migration while changing V2 section ownership.
- [x] Make hot reload reject invalid configuration visibly rather than continuing with stale policy.
- [x] Emit a root-reachable JSON Schema for the actual V2 YAML only; remove or relocate unreachable definitions that appear configurable.
- [x] Update bounded schema sections so `on`, `execution`, `hooks`, and `job` expose their real configurable fields and constraints.
- [x] Change schema identity/version independently from protocol and binary versions.
- [x] Prove catalog, parser allowlists, schema, defaults, and semantic validation stay in parity.
- [x] Cover happy and unhappy paths with focused unit tests before implementation.

## Outcome

V2 configuration now assigns event input to `on`, runtime policy to `execution`, and lifecycle commands to `hooks`. Preferred `jobs:` is strict; grouped legacy `tasks:` retains its historical policy placement only as an explicit compatibility boundary.

## Notes

Do not redesign runtime hook execution or control protocol. This task changes configuration ownership and validation boundaries.
