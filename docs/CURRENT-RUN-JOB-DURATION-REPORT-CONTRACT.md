# Current-Run Job Duration Report Contract

Report each finite job's duration from its terminal executor snapshot. Do not measure it again or sum job durations to obtain the generation total. A job that never started has no duration.

## 1. Scope

This contract applies to one Funzzy generation: local `fzz run`, foreground blocking watches, restart-capable worker completions, and correlated control observations. The report projects terminal job snapshots. It does not change scheduling or add historical statistics.

## 2. Authoritative measurement

`executor::TaskSnapshot.duration_ms` is the sole per-job measurement. It counts non-negative monotonic elapsed wall-clock milliseconds from the job's first successful spawn to its final terminal outcome.

A renderer MUST NOT start another timer, infer a duration from log or event
timestamps, add command durations, or replace a supplied value. Human
formatting is only a presentation of that integer; structured consumers retain
`durationMs` unchanged.

## 3. Job terminal meanings

A job row has one final state and at most one `durationMs` value:

| Final condition | State | `durationMs` |
| --- | --- | --- |
| All configured commands (and any required verification) pass | `passed` | terminal snapshot value |
| A command, spawn, or final verification fails | `failed` | terminal snapshot value |
| A started job is cancelled or superseded | `cancelled` | partial elapsed value at cancellation |
| A job never starts, including fail-fast-skipped work | `cancelled` | `null` |
| A finite hook has no job snapshot | not a job row | not reported |
| A legacy managed service remains alive | service/running, generation remains active | `null` |
| A readiness-enabled service reaches readiness | `passed` at promotion; later uptime is pool state | terminal snapshot value |

`null` means the executor has no finite duration for the job. Render it as `-` in a human report. Do not show `0ms`, estimate a value, or imply that a job ran when it did not. A measured `0` remains valid and differs from `null`.

A recovered job appears exactly once, in its configured position. Its duration
covers its original attempt, approval boundary, recovery commands, and final
verification through its final terminal outcome. It is not split into recovery
phase rows and a successful verification changes its one row to `passed`.

## 4. Generation total

Measure generation total separately with a monotonic wall clock, from run start to the terminal result. Do not sum job durations: serial work, parallel overlap, cancellation, recovery, and hooks make that sum a different measure. Existing aggregate outcome,
counts, failures, exit status, colors, and log mirroring retain their meanings.

## 5. Ordering and identity

Every finite configured job has at most one report row. Rows use the stable
configured job name and group identity already selected for the generation and
are ordered by configured declaration position, not completion time, duration,
or recovery phase. This rule is identical for serial and parallel plans.

A row's duration therefore remains meaningful independently of whether the
job ran alone or overlapped another job. The report carries no parallelism
caveat or reconstructed queue time.

## 6. Human and structured representation

A human terminal result renders one deterministic table after the existing
aggregate result, conceptually:

```text
PASS generation=42 total=12.4s

JOB             RESULT      DURATION
format-check    passed         0.7s
lint            failed         1.8s
docs            cancelled         -
```

Column padding may adapt to names, but row ordering, state labels, and the
dash for absent durations are deterministic. The shared duration formatter is
the canonical human rendering and never changes the underlying integer.

Structured snapshots and `task_terminal` events keep their additive `tasks[].durationMs` / `durationMs` integer-or-null field. New projections use the same immutable value; they do not measure again. A legacy snapshot without task data remains valid. It has no job-duration rows, and clients must treat availability as unknown rather than invent a value. Per-job duration availability is neither a freshness claim
nor a substitute for generation identity.

## 7. Services and hooks

A legacy `service: true` job without readiness has no completed lifetime while
it is alive. A report MUST represent it as running/service without a duration;
its unbounded uptime is never a completed job duration.

A readiness-enabled service has an explicit promotion boundary. Its service
row records the monotonic duration from service spawn through committed
readiness, and the generation can settle as `passed`. After promotion, the
service remains worker-owned and its uptime is live pool state, not a new
or changing generation duration. A later restart or failure does not rewrite
the settled generation's duration or outcome.

Generation hooks are not configured jobs and do not create job rows or change
job measurements.

## 8. Compatibility

This is additive presentation over existing terminal snapshots. Consumers that
already read `durationMs` keep its integer-or-null semantics. Consumers without
per-job snapshots continue to receive the existing aggregate generation result
and must not infer per-job timing from it. Control capability and freshness
negotiation are unchanged.

## 9. Non-goals

This contract does not add command-level or recovery-phase breakdowns, queue
wait time, CPU time, historical percentiles, regression detection, persisted
per-job reports, a second clock, or any change to duration-history estimates.
