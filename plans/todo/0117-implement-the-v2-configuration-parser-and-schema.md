---
id: TASK-0117
title: Implement the V2 configuration parser and schema
status: todo
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

- [ ] Write failing parser tests first for canonical V2, each new root section, existing V1 task-list compatibility, unknown keys, wrong types, malformed hooks, and the contract decision for previous grouped V2 field placements.
- [ ] Extend the catalog ownership model from `Root | On | Job` to include `Execution` and `Hooks`; keep one property identity and ordering source.
- [ ] Parse V2 `on`, `execution`, `hooks`, and ordered `jobs` into existing runtime policy types without changing job ordering, matching, hook lifecycle, socket, or execution behavior.
- [ ] Preserve existing V1 root task-list compatibility in ordinary runtime paths; do not broaden or redefine migration while changing V2 section ownership.
- [ ] Make hot reload reject invalid configuration visibly rather than continuing with stale policy.
- [ ] Emit a root-reachable JSON Schema for the actual V2 YAML only; remove or relocate unreachable definitions that appear configurable.
- [ ] Update bounded schema sections so `on`, `execution`, `hooks`, and `job` expose their real configurable fields and constraints.
- [ ] Change schema identity/version independently from protocol and binary versions.
- [ ] Prove catalog, parser allowlists, schema, defaults, and semantic validation stay in parity.
- [ ] Cover happy and unhappy paths with focused unit tests before implementation.

## Notes

Do not redesign runtime hook execution or control protocol. This task changes configuration ownership and validation boundaries.
