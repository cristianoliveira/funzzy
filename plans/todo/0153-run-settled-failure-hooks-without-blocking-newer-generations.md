---
id: TASK-0153
title: Run settled failure hooks without blocking newer generations
status: doing
depends_on: [TASK-0152]
priority: high
tags: [rust, config, hooks, scheduler, process, tdd]
---

# Run settled failure hooks without blocking newer generations

## Problem
Users need their existing arbitrary failure-hook command to run only after the failed outcome remains current for a configured settle period, without delaying newer watcher work.

## Outcome

Implement the accepted TASK-0152 contract through tests first while preserving immediate scalar hooks.

## Acceptance criteria

- [ ] Parser and runtime model accept the contracted object form and retain scalar compatibility.
- [ ] Empty commands, zero/negative/unsupported durations, unknown fields, and incompatible declarations fail during configuration validation.
- [ ] A failed generation publishes its terminal outcome without waiting for the settle duration.
- [ ] The custom command runs once after the full settle period only when that failed generation is still latest and no replacement is active or queued.
- [ ] Newer generation scheduling atomically cancels a pending settled hook before replacement work proceeds.
- [ ] Repeated failures coalesce toward the newest generation rather than executing stale commands.
- [ ] A newer pass suppresses every older pending failure command.
- [ ] Pending and running settled-hook processes follow existing cancellation, process-group ownership, output, and reaping policies.
- [ ] Hook command failure remains observable and cannot replace the generation outcome.
- [ ] Configuration revision hashing/reload treats settle policy as semantic and keeps each pending hook bound to its generation snapshot.
- [ ] Unit tests cover happy and unhappy paths, including timer/new-generation race with an injected deterministic clock or equivalent controlled boundary.

## Constraints

- Do not use fixed sleeps as the assertion strategy.
- Do not introduce an agent, notification, or platform dependency.
- Do not preserve a second deprecated configuration path.

## Notes

Follow the contract produced by TASK-0152; do not infer unresolved lifecycle semantics during implementation.

