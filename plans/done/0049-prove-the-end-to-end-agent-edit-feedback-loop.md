---
id: TASK-0049
title: Prove the end-to-end agent edit feedback loop
status: done
depends_on: [TASK-0034, TASK-0045, TASK-0046, TASK-0048, TASK-0050]
priority: high
tags: [integration-tests, axi, control-socket, reliability, performance]
---

# Prove the end-to-end agent edit feedback loop

## Problem
Individual protocol features do not prove an agent can edit, trigger or observe, await exact fresh verification, diagnose failure, cancel obsolete work, and recover with bounded tool calls.

## Context

Use black-box watcher plus client scenarios, isolated workspaces, deterministic fixture commands, and bounded timeouts. Measure tool round trips and response size.

## Acceptance criteria

- [ ] Scenario observes baseline, edits file, awaits exact resulting generation, and proves green is fresh for latest batch.
- [ ] Failure scenario returns task-attributed bounded evidence, retrieves detail, edits fix, and awaits recovery without parsing human logs.
- [ ] Rapid two-edit scenario proves first is superseded/stale and second result cannot be confused with first.
- [ ] Cancellation scenario kills descendant process tree and newer generation remains unaffected.
- [ ] Config reload/watcher restart scenario returns explicit instance change rather than false terminal result.
- [ ] No-match, ignored path, timeout, disconnect, malformed request, truncated output, and old-server capability paths are covered.
- [ ] Parallel completion order does not change combined structured outcome semantics.
- [ ] Common successful loop needs at most status plus one await, or one run/emit-with-wait call; response fixtures stay within declared token/byte budget.
- [ ] Rust socket integration and Pi watcher consumer tests pass under Rust 1.97 environment.
- [ ] Documentation provides copyable agent integration examples and states freshness guarantees/limits.

## Notes

