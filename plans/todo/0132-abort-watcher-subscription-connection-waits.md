---
id: TASK-0132
title: Abort watcher subscription connection waits
status: todo
depends_on: []
priority: high
tags: [typescript, pi-watcher, socket, observer, timeout, cancellation, tdd]
---

# Abort watcher subscription connection waits

## Problem
watcher_observe owns a timeout AbortSignal, but the subscription adapter waits for Unix-socket connection without observing it. A stalled handshake can outlive caller timeout and leave an agent tool stuck despite the bounded application policy.

## Context

`requestObservation` aborts its internal signal at deadline, and `socketJsonLines` already consumes that signal after connection. However, `subscribeToSnapshots` first awaits `waitForConnect(socket)` without passing the signal. Make the handshake abort-aware and clean up every listener/socket deterministically.

## Acceptance criteria
- [ ] Add a failing infra test where connection never reaches `connect` and abort settles the iterator promptly.
- [ ] Pass `AbortSignal` into the connection wait and reject with a stable cancellation error when already or subsequently aborted.
- [ ] Remove `connect`, `error`, and `abort` listeners on every resolve/reject path.
- [ ] Ensure `subscribeToSnapshots` destroys the socket after handshake abort and does not write the subscribe request.
- [ ] Preserve connection-error classification and successful subscription behavior.
- [ ] Prove application timeout returns `timeout`, not `disconnect`/`unknown`, when abort interrupts handshake.
- [ ] Cover already-aborted signal and abort-vs-connect race deterministically without sleep.
- [ ] Run focused infra/application tests, pi-watcher static checks, and fresh final watcher gate.

## Notes

Keep scope to subscription connection cancellation. Do not change observe selector semantics, retry policy, or default timeouts.

