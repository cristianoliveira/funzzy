---
id: TASK-0050
title: Expose correlated watcher lifecycle subscriptions
status: done
depends_on: [TASK-0042, TASK-0043, TASK-0044, TASK-0047]
priority: high
tags: [rust, control-socket, subscription, events, pi-watcher, tdd]
---

# Expose correlated watcher lifecycle subscriptions

## Problem
pi-watcher already consumes a long-lived `subscribe` stream, but the Rust server roadmap only guarantees atomic await; without an explicit server task, capability negotiation has no matching push lifecycle implementation to advertise.

## Context

Add optional JSON-RPC `subscribe` over existing NDJSON Unix socket. It returns one immediate correlated snapshot, then emits `snapshot` notifications on meaningful lifecycle transitions. Reuse same snapshot/event source as atomic await; do not build second state tracker.

Wire shape consumed by pi-watcher:

```text
request:      {"jsonrpc":"2.0","id":"subscribe","method":"subscribe"}
response:     {"jsonrpc":"2.0","id":"subscribe","result":<snapshot>}
notification: {"jsonrpc":"2.0","method":"snapshot","params":<snapshot>}
```

## Acceptance criteria

- [ ] Contract tests first lock request, initial response, notification framing, malformed parameters, and backward-compatible unknown-method behavior.
- [ ] Subscription registration and initial snapshot are atomic: no lifecycle transition can be lost between snapshot read and listener registration.
- [ ] Initial result and every notification use exact correlated snapshot schema from TASK-0043, including instance, batch, generation, pending, task outcomes, and freshness.
- [ ] Notifications cover batching/queued/running/terminal/superseded/cancelled transitions defined by TASK-0042 without emitting duplicate snapshots for unchanged state.
- [ ] Multiple subscribers observe same monotonic transition sequence without blocking watcher scheduling or each other.
- [ ] Per-subscriber buffering is bounded; slow consumers are disconnected with explicit policy rather than growing memory or stalling executor.
- [ ] Client disconnect, watcher shutdown, config reload, and write failure remove subscriber and release all tasks/channels promptly.
- [ ] `capabilities` advertises method `subscribe` and feature `subscription: true` only when endpoint is registered; legacy fields remain unchanged.
- [ ] Subscription and `await` share one injected event publisher and identical freshness semantics.
- [ ] Rust socket integration tests and pi-watcher `infra/observer.ts` fixtures prove immediate snapshot, pushed transitions, reconnect, and watcher instance change.
- [ ] Protocol documentation states ordering scope, delivery guarantee, backpressure, reconnect, and that clients must resume from new atomic snapshot after disconnect.

## Notes

This deliberately adds subscription in addition to atomic await because pi-watcher already implements long-lived push observation. If TASK-0042 chooses long-poll-only instead, retire this task and change pi-watcher before server implementation rather than supporting two accidental contracts.
