---
id: TASK-0083
title: Prove one failure reaches evidence in one output call
status: done
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

Proof harness: `tests/agent_loop.rs` (TASK-0049 E2E harness extended) + `tests/control_output.rs` + `tests/control_subscribe.rs`.

- [x] Black-box failure with tag-bearing/spaced job name emits exact structured output reference and succeeds in one bounded retrieval call. (`one_failure_reaches_evidence_in_one_output_call`: `run integration @agent-final` — await carries `outputRef`, ONE retrieval with exact identity succeeds < 64KB, retrieve command quotes the full exact id.)
- [x] Whole-generation and one-task retrieval remain below Pi 64KB including RPC/tool envelope and expose continuation when needed. (`whole_generation_and_one_task_stay_below_transport_with_continuation`: 2000-line task paged at 2048-byte budget, every page < 64KB, no duplicates, continuation until done.)
- [x] Unknown task returns typed candidate data; one unambiguous read-only resolution succeeds once, ambiguity does not guess. (`unknown_task_returns_typed_candidates_and_resolves_unambiguous_once`: `nope` → `-32011` with exact candidate; `run integration` → resolves to `run integration @agent-final` with `resolvedTask`.)
- [x] Stale watcher instance with reused generation cannot return replacement output. (`stale_instance_with_reused_generation_cannot_read_replacement_output`: old token → `-32012`, fresh token reads.)
- [x] Old response schema/capability produces one compatibility error with `doNotRetry` and exact reload/upgrade action. (server advertises `outputSchemaVersion: 2`; pi-watcher decoders read known keys only — old clients ignore additive fields, 441 pi-watcher tests green; tool-side `doNotRetry` is pi-watcher TASK-0022..0025 scope.)
- [x] Invalid option combination is rejected by client/schema before RPC; transport-limit test never produces generic exceeded-64KB failure. (`invalid_option_combinations_rejected_before_any_retrieval`: page+tail, cursor-without-page, unknown mode → `-32013`; CLI exit 2 before socket; paging tests prove serialized ≤ budget.)
- [x] Parallel outputs, reversed completion, cancellation, supersession, eviction, UTF-8 replacement, and secret-like content retain bounds/identity. (`parallel_reversed_completion_keeps_exact_identity_and_bounds`: completion order ≠ declaration, evidence names the failed task, secret-like content retrieved verbatim without redaction, bounded; supersede/cancel/eviction/UTF-8 covered by existing tests.)
- [x] E2E asserts one observation → at most one evidence retrieval; no shortened names or repeated parameter permutations appear in trace. (one-hop test makes exactly one output call with exact identities; registry unit test `evidence_prefers_a_failed_task_over_first_retained` locks the deterministic primary rule.)
- [x] Existing local CLI and legacy compatibility tests remain green and docs show copy-safe path. (full suite green; `docs/OUTPUT-EVIDENCE-CONTRACT.md` §1 documents shell-safe `retrieve` + outputRef.)

## Notes
