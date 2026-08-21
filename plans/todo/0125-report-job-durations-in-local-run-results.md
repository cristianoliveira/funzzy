---
id: TASK-0125
title: Report job durations in local run results
status: todo
depends_on: [TASK-0124]
priority: high
tags: [rust, cli, duration, jobs, output, tdd]
---

# Report job durations in local run results

## Problem
`fzz run` and foreground execution currently finish with aggregate generation duration, while per-job terminal durations measured by the executor are not carried into the human result summary.

## Context

Extend result projection rather than timing jobs again. Same executor events/outcomes must drive serial, parallel, recovery, failure, and cancellation presentation.

## Acceptance criteria

- [ ] Write failing presentation/domain tests first for passed, failed, serial, parallel, cancelled, skipped, and recovered jobs.
- [ ] Carry existing terminal job snapshots or equivalent immutable report value into `CompletedRun`/result presentation without coupling stdout to watcher state.
- [ ] Do not add a second timer; every displayed duration must originate from executor monotonic measurement.
- [ ] Render one deterministic row per configured job in declaration order for both serial and parallel plans.
- [ ] Show job name, final state, and readable duration; use a dash when duration is absent and keep full integer milliseconds available to structured consumers.
- [ ] Keep generation total separate from job rows and preserve existing success/failure counts, errors, exit code, coloring, and log mirroring.
- [ ] Ensure recovered jobs report duration through final verification and appear once with final state, not once per recovery phase.
- [ ] Ensure started cancellations report partial elapsed duration and never-started/skipped jobs report no fabricated zero.
- [ ] Share one report projection across local `fzz run`, blocking watch, and restart-capable worker completion paths rather than duplicating formatting policy.
- [ ] Keep output deterministic under parallel completion order by sorting through configured job position.
- [ ] Cover happy and unhappy paths with focused tests before implementation.

## Notes

Do not redesign historical duration estimates or introduce command-level timing in this task.
