---
id: TASK-0079
title: Define one-hop agent output evidence contract
status: doing
depends_on: [TASK-0042, TASK-0045, TASK-0047, TASK-0048]
priority: high
tags: [design, axi, control-socket, output, agents, compatibility]
---

# Define one-hop agent output evidence contract

## Problem
A real Pi session made eight unsuccessful watcher_output calls because evidence identity, schema compatibility, parameter combinations, and transport bounds were not self-correcting.

## Context

Ground contract in `.tmp/reports/15-08-26/watcher-output-agent-confusion-session-audit.md`. Goal is one notification/observation followed by at most one successful retrieval call, without agent-generated identity text.

## Acceptance criteria

- [ ] Defines structured `outputRef` containing watcher instance token, generation, exact task ID when narrowed, and safe retrieval defaults/budget.
- [ ] Defines whole-generation and one-task references, stable serialization, lifecycle, eviction, restart, supersession, and cancellation semantics.
- [ ] Capability advertises output response schema version, supported request variants, paging model, and effective response-byte limit; boolean `outputRetrieval` alone is insufficient for advanced client.
- [ ] Typed RPC errors distinguish instance mismatch, unknown/evicted generation, unknown task, invalid cursor/options, unavailable output, and internal failure, with stable codes/data.
- [ ] Exact task ID is machine identity; display job name/tags are metadata and never reconstructed from prose.
- [ ] `tail` and page/full semantics cannot coexist ambiguously; contract defines precedence by rejecting invalid shape before transport.
- [ ] Every successful response is below negotiated agent budget and reports observed/retained/returned bytes, truncation, and continuation explicitly.
- [ ] Read-only canonicalization/auto-retry is allowed only for one structured, unambiguous candidate and must report selected exact ID; ambiguity schedules no guess.
- [ ] Compatibility failure says upgrade/reload action and `doNotRetry: true`; no parameter permutation can turn schema mismatch into valid response.
- [ ] Security section preserves 0600 socket boundary, secret-bearing output caveat, and bounded evidence defaults.

## Notes

This supersedes assumption that server-retention bound alone makes one RPC response agent-safe.
