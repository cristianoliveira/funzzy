---
id: TASK-0164
title: Implement watch exclusions and no-services mode
status: todo
depends_on: [TASK-0163]
priority: high
tags: [rust, cli, targets, services, worktrees, tdd]
---

# Implement watch exclusions and no-services mode

## Problem
Funzzy needs a deterministic invocation-level way to run the configured workflow except selected jobs, including a simple shortcut that excludes every service: true job.

## Desired outcome

The command contract from TASK-0163 is available through the real CLI and shared planning path, with excluded jobs removed before execution can acquire process ownership.

## Acceptance criteria

- [ ] Add parser/help support for the approved `--exclude TARGET` and `--no-services` surfaces on every command approved by TASK-0163.
- [ ] Resolve exclusions through existing target-selection rules rather than creating a second target grammar.
- [ ] Filter excluded jobs while preserving remaining declaration order, group boundaries, signatures, and path matching.
- [ ] Ensure `--no-services` filters both legacy and readiness-enabled service jobs before spawn, readiness, handoff, or pool reconciliation.
- [ ] Compose positive target selection, repeated exclusions, and `--no-services` exactly as the contract specifies.
- [ ] Return approved actionable errors for ambiguous, unmatched, or empty selections.
- [ ] Preserve byte/behavior compatibility when neither exclusion option is present.
- [ ] Add focused happy and unhappy unit tests before implementation changes.

## Non-goals

- Coordinating service ownership across watcher processes.
- Persisting exclusions into `.watch.yaml` or hot-reload revisions.
- Adding service-specific telemetry.

## Constraints

Use the shared planning/filtering boundary. Do not scatter CLI exclusion checks through executor or worker lifecycle code.

