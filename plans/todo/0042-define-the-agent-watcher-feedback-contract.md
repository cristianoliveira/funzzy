---
id: TASK-0042
title: Define the agent watcher feedback contract
status: doing
depends_on: [TASK-0014]
priority: high
tags: [design, axi, control-socket, protocol, determinism]
---

# Define the agent watcher feedback contract

## Problem
Agent integrations currently have no explicit contract for proving that a terminal result is fresh for a specific edit, so polling, logs, and timestamps can produce races and false confidence.

## Context

Specify state machine before extending JSON-RPC or CLI. Core loop is `observe -> edit -> await exact fresh generation -> diagnose -> act`. Preserve existing protocol fields additively.

## Acceptance criteria

- [ ] Contract defines watcher instance, event batch, generation, task, command, and group-occurrence identities and their lifetimes.
- [ ] State transitions cover idle, batching, queued, running, terminal, superseded, cancelled, timed out, watcher restart, and disconnect.
- [ ] Freshness rule states when a result proves latest observed filesystem state and when state is stale/unknown.
- [ ] Atomic snapshot and `await --after GENERATION` semantics eliminate subscribe-after-read race.
- [ ] Run/emit wait behavior, no-match/no-op, timeout, and reconnect semantics are explicit.
- [ ] Output evidence, truncation, redaction, retention, cancellation, and schema negotiation boundaries are defined.
- [ ] JSON-RPC compatibility policy is additive and distinguishes protocol JSON from optional CLI TOON rendering.
- [ ] Deterministic exit-code matrix covers success/no-op, workflow/operational failure, usage error, timeout, cancel, supersede, and disconnect.
- [ ] Black-box contract matrix is recorded before implementation tasks proceed.

## Notes

See `.tmp/reports/13-04-26/llm-agent-watcher-needs.md`.

