---
id: TASK-0048
title: Render agent-safe structured control responses
status: todo
depends_on: [TASK-0039, TASK-0044, TASK-0047]
priority: high
tags: [rust, cli, axi, toon, json, output, tdd]
---

# Render agent-safe structured control responses

## Problem
Human-oriented control output is expensive and unstable for agents; status, await, run, emit, output, and errors need deterministic compact structured representations with clean stream separation.

## Context

Keep domain responses format-independent. Encode structured CLI output once at boundary using maintained TOON implementation and current spec; JSON remains interoperability option. Human mode remains explicit.

## Acceptance criteria

- [ ] Representative status, await, list, run, emit, cancel, output, capabilities, empty/no-op, usage, operational error, and truncation fixtures are defined first.
- [ ] `--format toon|json|human` contract and default for TTY/non-TTY are explicit; no terminal-width-dependent structured output.
- [ ] Structured stdout contains only one valid response/document or declared NDJSON stream; progress/debug stays stderr.
- [ ] Errors use same selected structured format and stable code; exit status is 0 success/no-op, 1 workflow/operational, 2 usage.
- [ ] Unknown inputs name offending value and compact valid alternatives without contacting socket when validation can be local.
- [ ] TOON uses maintained spec-compatible library, round-trips semantically, escapes correctly, and is measured against JSON on representative payloads.
- [ ] Large fields expose preview, total size, truncation, and copyable retrieval command only when needed.
- [ ] Tests prove deterministic ordering, secret redaction policy, clean streams, broken pipe, and schema parity across formats.

## Notes

