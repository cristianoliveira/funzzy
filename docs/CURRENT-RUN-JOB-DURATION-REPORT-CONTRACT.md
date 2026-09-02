# Current-Run Job Duration Report Contract

## 1. Scope

This contract defines per-job duration reporting for one Funzzy generation.
It applies to local `fzz run` results, foreground blocking watches,
restart-capable worker completions, and correlated control observations.

A report is a projection of the executor's terminal job snapshots. It does not
measure work itself, alter scheduling, or add historical statistics.

## 2. Authoritative measurement

`executor::TaskSnapshot.duration_ms` is the sole authoritative per-job
measurement. Its value is a non-negative integer count of monotonic elapsed
wall-clock milliseconds from the first successful spawn for that job until the
job's final terminal outcome.

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

`null` means the job did not obtain a finite executor duration. A human report
renders it as `-`; it MUST NOT render `0ms`, estimate a value, or imply that a
never-started job ran. A measured `0` remains a valid integer and is distinct
from `null`.

A recovered job appears exactly once, in its configured position. Its duration
covers its original attempt, approval boundary, recovery commands, and final
verification through its final terminal outcome. It is not split into recovery
phase rows and a successful verification changes its one row to `passed`.

## 4. Generation total

The generation total is a separate monotonic wall-clock measurement from
run start until the generation's terminal result. It MUST NOT be calculated by
summing job durations: serial work, parallel overlap, cancellation, recovery,
and hooks make such a sum semantically different. Existing aggregate outcome,
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

Structured snapshots and `task_terminal` events retain their additive
`tasks[].durationMs` / `durationMs` integer-or-null field. New result
projections expose the same immutable report value rather than a duplicate
measurement. A legacy snapshot that lacks task data remains valid: it has no
job-duration rows, and clients MUST treat availability as unknown rather than
inventing a value. Per-job duration availability is neither a freshness claim
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
