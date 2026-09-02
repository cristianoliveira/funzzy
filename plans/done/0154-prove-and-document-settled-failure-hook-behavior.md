---
id: TASK-0154
title: Prove and document settled failure hook behavior
status: done
depends_on: [TASK-0153]
priority: high
tags: [integration-tests, docs, hooks, config, watcher, reliability]
---

# Prove and document settled failure hook behavior

## Problem
Delayed failure-hook behavior crosses scheduling, cancellation, finite runs, reloads, compatibility, and external evidence forwarding. Today users can discover `hooks.failure.run` and `settle`, but cannot discover whether the hook receives an exact generation ID, failed job names, retained evidence, or output through argv, environment, or stdin. Without black-box proof and a complete invocation contract, integrations must inspect implementation logs and may forward stale or miscorrelated failures.

## Outcome

Prove the accepted contract at the CLI and watcher boundaries and teach users when to choose immediate versus settled custom failure hooks.

## Acceptance criteria

- [x] Black-box test proves a stable failed generation runs the custom command exactly once after the settle boundary.
- [x] Black-box test proves a newer generation can start before the old settle duration expires.
- [x] Tests prove newer pass suppression, repeated-failure coalescing, cancellation/supersession, watcher shutdown, and custom-command failure.
- [x] Tests cover the contracted finite-run, control-await, valid reload, and malformed-reload behavior.
- [x] Existing immediate scalar success/failure hook tests remain green and demonstrate compatibility.
- [x] Schema, generated config example, `fzz check`, README/USAGE, and `docs/RUN-HOOKS-CONTRACT.md` agree on syntax and lifecycle.
- [x] Documentation says settlement is based on watcher generations, not knowledge of agent activity.
- [x] Document the complete hook invocation contract: shell/argv behavior, workspace or configured working directory, stdin, inherited and Funzzy-provided environment, output capture, exit handling, and process lifetime.
- [x] State exactly how the hook correlates to the immutable failed generation and its failed jobs/evidence. Do not claim the command receives generation identity when it exists only in internal diagnostics or events.
- [x] Provide exact immutable correlation through reserved `FUNZZY_GENERATION_ID` and `FUNZZY_GENERATION_OUTCOME` environment variables for immediate and settled generation hooks; Funzzy values override inherited values, while exact retained evidence remains available through `fzz control output --generation "$FUNZZY_GENERATION_ID"`.
- [x] Either prove the hook receives an exact immutable generation identifier suitable for `fzz control output --generation`, or explicitly document that it does not and label any `control status` latest-generation lookup limitation/race.
- [x] Document output-retention guarantees and the difference between latest `control status` and exact `control output --generation N` retrieval.
- [x] Add a self-contained evidence-forwarding recipe: wait for a settled failure, obtain the exact retained result, and invoke a user-owned transport. Include a Pi Bebop socket example using `pi-bebop send --socket .pi/bebop/sockets/dev.sock --mode steer --wait accepted` without making Pi a Funzzy dependency.
- [x] Documentation warns that external side effects cannot be recalled once command execution begins.
- [x] Verification includes focused tests plus configured final watcher gates; failure evidence is retained in the task notes.

## Constraints

- Use bounded polling or lifecycle events in integration tests, never timing-only fixed sleeps.
- Keep examples generic: the command may call any user-owned script.

## Notes

QA should challenge boundary races rather than treating elapsed wall time alone as proof.

Feedback captured 2026-09-01: a user configured `failure.settle: 3m` and had to infer a `control status` → `control output --generation` → Pi Bebop forwarding flow from logs and binary strings because the hook payload and working-directory contract were undiscoverable. Treat this as a documentation defect and a possible exact-correlation capability gap, not as a request for built-in Pi integration.

Implementation evidence (2026-09-02): `tests/settled_failure_hooks.rs` provides nine feature-gated black-box tests for stable settlement, exact-once invocation, reserved environment correlation, supersession/coalescing/pass suppression, shutdown, hook failure, finite execution, and valid/malformed reloads. `cargo test --test settled_failure_hooks --features test-integration -- --nocapture` passed (9 tests). Immediate scalar compatibility passed via `cargo test --test run_once --features test-integration success_and_failure_hooks_run_once_per_generation -- --nocapture`. Schema/init/CLI unit suites and `cargo test --lib` passed. Configured final watcher gates subsequently passed: generation 16 `integration @agent-final` (447453ms), generation 17 `format @quick @agent-final` (2433ms), and generation 23 `unit tests @quick @agent-final` (64921ms), all with unchanged worktree fingerprint `35875274d1a7`. The external watcher was available for these explicit runs.
