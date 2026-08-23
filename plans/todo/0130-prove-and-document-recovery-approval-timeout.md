---
id: TASK-0130
title: Prove and document recovery approval timeout
status: doing
depends_on: [TASK-0129]
priority: high
tags: [integration-tests, docs, recovery, approval, timeout, watcher, reliability]
---

# Prove and document recovery approval timeout

## Problem
Users and agent clients need executable proof and discoverable guidance that forgotten recovery confirmation cannot block watcher completion indefinitely.

## Context

Prove observable behavior through spawned CLI/watcher boundaries, not only executor fakes. Agent success condition: exact-generation await returns terminal failure after approval budget instead of remaining running forever.

## Acceptance criteria
- [x] Add PTY-backed integration proof that unanswered prompt times out, runs no recovery command, and exits/finalizes as failure.
- [x] Prove an affirmative answer before deadline still performs one recovery and one verification.
- [ ] Prove cancellation/supersession before deadline wins and late or partial input cannot approve another generation.
- [ ] Prove control status/await observes one non-terminal approval phase followed by exact-generation terminal failure with timeout evidence.
- [x] Update README, init/example configuration, canonical schema/help, and recovery contract with default and override example.
- [x] Confirm pi-watcher needs no protocol change beyond continuing to wait for final terminal event; add/update e2e regression if timeout evidence decoding changes.
- [ ] Run focused Rust tests, integration gate, and pi-watcher checks through configured watcher targets.

## Notes

Avoid sleep-based threshold assertions. Synchronize on emitted `approval_requested`, then assert bounded terminal transition with generous outer test timeout.

Reopened during TASK-0131 review: configuration and basic PTY timeout are complete, but late/partial-input isolation and control-socket terminal-transition proof were not present.

