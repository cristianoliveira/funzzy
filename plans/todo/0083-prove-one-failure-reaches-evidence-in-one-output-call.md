---
id: TASK-0083
title: Prove one failure reaches evidence in one output call
status: todo
depends_on: [TASK-0082, TASK-0049]
priority: high
tags: [integration-tests, axi, output, agents, compatibility, reliability]
---

# Prove one failure reaches evidence in one output call

## Problem
Unit protocol tests cannot prove an agent receives a failed exact generation, follows its output reference once, stays under transport budget, and avoids stale or guessed task evidence.

## Context

Replay failure pattern from audited Pi session with deterministic fixture and record tool-call count, not subjective agent interpretation.

## Acceptance criteria

- [ ] Black-box failure with tag-bearing/spaced job name emits exact structured output reference and succeeds in one bounded retrieval call.
- [ ] Whole-generation and one-task retrieval remain below Pi 64KB including RPC/tool envelope and expose continuation when needed.
- [ ] Unknown task returns typed candidate data; one unambiguous read-only resolution succeeds once, ambiguity does not guess.
- [ ] Stale watcher instance with reused generation cannot return replacement output.
- [ ] Old response schema/capability produces one compatibility error with `doNotRetry` and exact reload/upgrade action.
- [ ] Invalid option combination is rejected by client/schema before RPC; transport-limit test never produces generic exceeded-64KB failure.
- [ ] Parallel outputs, reversed completion, cancellation, supersession, eviction, UTF-8 replacement, and secret-like content retain bounds/identity.
- [ ] E2E asserts one observation → at most one evidence retrieval; no shortened names or repeated parameter permutations appear in trace.
- [ ] Existing local CLI and legacy compatibility tests remain green and docs show copy-safe path.

## Notes
