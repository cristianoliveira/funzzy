---
id: TASK-0004
title: Runtime-validate the Funzzy control protocol in pi-watcher
status: todo
depends_on: []
priority: high
tags: [pi-watcher, protocol, typescript]
---

# Runtime-validate the Funzzy control protocol in pi-watcher

## Problem

Rust serializes control state while TypeScript manually duplicates its shape and trusts generic casts. Compatible-looking protocol changes can compile on both sides and fail only at Pi runtime.

## Scope

- `pi-watcher/src/infra/client.ts`
- `pi-watcher/src/domain/watcher.ts`
- Client contract tests and representative Rust fixtures

## Acceptance criteria

- [ ] JSON-RPC envelope remains validated.
- [ ] `status`, `targets`, and `run` results decode from `unknown` with explicit runtime checks.
- [ ] Missing, extra-incompatible, or wrong-type fields produce actionable errors.
- [ ] Valid Rust-produced payload fixtures decode successfully.
- [ ] Public socket methods and payload compatibility remain unchanged.
- [ ] Contract coordination requirement is documented near both boundaries.

## Verification

- Tests cover valid and malformed payloads for every method.
- Pi watcher format, lint, typecheck, coverage, and audit gates pass.

