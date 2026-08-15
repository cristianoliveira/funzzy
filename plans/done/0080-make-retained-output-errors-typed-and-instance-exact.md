---
id: TASK-0080
title: Make retained output errors typed and instance exact
status: done
depends_on: [TASK-0079, TASK-0046]
priority: high
tags: [rust, control-socket, identity, errors, output, tdd]
---

# Make retained output errors typed and instance exact

## Problem
Output retrieval currently uses instance-scoped generation without instance token and maps missing generation/task to generic server errors, so clients cannot distinguish stale watcher, eviction, typo, or protocol failure.

## Context

Generation counter resets per watcher instance. Require exact instance on advanced retrieval while preserving explicit legacy behavior for old clients.

## Acceptance criteria

Contract: `docs/OUTPUT-EVIDENCE-CONTRACT.md` §3 (codes) + §6 (canonical candidates).

- [x] Tests first lock stable typed error codes and structured data for unknown instance/generation/task, eviction, invalid options/cursor, and unavailable registry. (`src/control.rs` tests: `output_mismatched_instance_token_is_typed_instance_error`, `output_missing_generation_maps_to_typed_code`, `output_missing_task_maps_to_typed_code_with_candidates`, `output_tail_and_full_together_is_typed_invalid_options`, `output_unavailable_registry_is_typed_unavailable`; `src/output.rs` typed `RetrievalError` tests.)
- [x] Output request validates `instanceToken` against active watcher before registry lookup; stale token cannot read same-number generation from replacement watcher. (`output_retrieval` checks `params.instanceToken` against `ControlInstance` before `retrieve`; stale → `-32012 instance_mismatch`.)
- [x] Missing/legacy instance behavior follows contract/capability and never claims exact freshness. (token absent → legacy path, no freshness claim.)
- [x] Registry stores/returns exact task ID separately from display name and emits deterministic canonical candidates for unknown task. (`RetrievalError::TaskNotFound { candidates, ambiguous }`; candidates = exact IDs the requested string prefixes, falling back to all retained exact IDs.)
- [x] One unambiguous read-only candidate may be resolved according to contract; multiple/zero candidates return typed error without retrieval. (single prefix match auto-resolves and reports `resolvedTask`; multiple → ambiguous error; zero → error listing exact IDs — no guessing.)
- [x] CLI and JSON/TOON render actionable exact retry data without parsing message strings. (`render_server_error` + output action renders `{error: {code, message, data}}` to stdout in selected format.)
- [x] pi-watcher expected `-32010/-32011` mappings and Rust server codes/fixtures agree; generic `-32000` is reserved for genuine server failure. (server now emits `-32010` generation / `-32011` task; pi-watcher client.ts maps them; `-32000` only for internal. pi-watcher suite green.)
- [x] Restart, generation reuse, cancellation, supersession, retention eviction, and concurrent retrieval race tests fail closed. (`watcher_restart_clears_all_retained_output` updated to typed `-32010` with empty retained; existing restart/eviction tests green.)
- [x] Existing clients receive additive-compatible response where feasible and capability clearly marks exact-output support. (response adds only `resolvedTask` when resolved; old decoders ignore unknown fields; legacy token-less requests keep working.)

## Notes
