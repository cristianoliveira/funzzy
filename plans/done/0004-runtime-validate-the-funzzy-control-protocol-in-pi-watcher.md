---
id: TASK-0004
title: Runtime-validate the Funzzy control protocol in pi-watcher
status: done
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

- [x] JSON-RPC envelope remains validated.
- [x] `status`, `targets`, and `run` results decode from `unknown` with explicit runtime checks.
- [x] Missing, extra-incompatible, or wrong-type fields produce actionable errors.
- [x] Valid Rust-produced payload fixtures decode successfully.
- [x] Public socket methods and payload compatibility remain unchanged.
- [x] Contract coordination requirement is documented near both boundaries.

## Verification

- [x] Tests cover valid and malformed payloads for every method.
- [x] Pi watcher format, lint, typecheck, coverage, and audit gates pass.
