---
id: TASK-0124
title: Define current-run job duration report contract
status: todo
depends_on: []
priority: high
tags: [design, duration, jobs, reporting, cli, determinism]
---

# Define current-run job duration report contract

## Problem
Funzzy already measures terminal duration for each job, but users cannot consistently see those measurements in run summaries and human control output, so slow jobs remain difficult to identify.

## Context

This slice reports one generation. It does not add historical per-job statistics or a second timing mechanism.

```text
PASS generation=42 total=12.4s

JOB             RESULT      DURATION
format-check    passed         0.7s
lint            passed         1.8s
test            passed        11.8s
```

Each duration belongs to that job regardless of serial or parallel execution. Generation total is a separate wall-clock measurement and is never derived by adding job durations.

## Acceptance criteria

- [ ] Publish a normative current-run report contract covering local runs, foreground watches, and correlated control observations.
- [ ] Reuse executor `TaskSnapshot.duration_ms` as authoritative job measurement; do not introduce another clock or reconstruct duration from log timestamps.
- [ ] Define job duration as monotonic elapsed wall time from job start until its final terminal outcome, including its full bounded recovery lifecycle when recovery occurs.
- [ ] Define generation total as separate wall-clock duration; never derive it by adding job durations.
- [ ] Apply identical job-duration meaning to serial and parallel jobs; do not add parallel-only caveats to individual job rows.
- [ ] Define passed/failed durations, partial duration for started-and-cancelled jobs, and `null`/dash for jobs that never started or were skipped.
- [ ] Define finite reporting behavior for services and hooks without presenting an unbounded service lifetime as a completed job duration.
- [ ] Preserve configured declaration order and stable job/group identity in every report.
- [ ] Define deterministic human formatting and structured integer `durationMs`; formatting never changes measured value.
- [ ] Keep existing structured snapshot fields additive-compatible and explicitly define legacy behavior when per-job measurements are unavailable.
- [ ] State non-goals: command-level breakdown, recovery-phase breakdown, queue time, CPU time, historical percentiles, regression detection, and persisted reports.

## Notes

Current evidence already exists in `src/executor.rs`, `src/snapshot.rs`, and `task_terminal` NDJSON events. This task locks presentation semantics before changing output.
