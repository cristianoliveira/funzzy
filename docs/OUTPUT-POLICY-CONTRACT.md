# Funzzy Task Output Policy Contract

> Status: **normative** — defined by TASK-0041. Per-job output policy over the
> global streaming default; composes with RUN-EVENTS-CONTRACT (machine
> export), the log file, and retained output retrieval (TASK-0045).

## 1. Policies

Each job may declare `output:` with one of:

| Policy | Live stdout/stderr | Retained capture | Use case |
| --- | --- | --- | --- |
| `inherit` (default) | streamed live, task-attributed | yes (bounded, for retrieval) | today's behavior |
| `quiet` | suppressed live | yes (bounded) | noisy jobs; diagnose on demand via `control output` |
| `capture` | not streamed; held until terminal | yes (bounded) | post-process the whole output |
| `show-on-failure` | suppressed live while passing; streamed (or revealed) exactly once if the job fails | yes (bounded) | CI-style "only show failures" |

`inherit` is the default, so **legacy jobs keep today's output**; only jobs
that declare a policy change behavior.

## 2. Bounds and truncation

- Captured output (all policies) uses the bounded capture buffer
  (TASK-0045): newest bytes up to a fixed cap, truncation always marked.
- Large output can never exhaust memory; the cap is per stream and shared
  with retained retrieval.
- Partial final lines (no trailing newline) and non-UTF8 bytes are handled
  lossily in live output and raw in capture, matching current behavior.

## 3. Line atomicity and attribution

- Every policy keeps output line-atomic and task-attributed for parallel
  jobs (TASK-0028): one whole line per write, `[task]` prefix for group
  members.
- Mixed sibling policies in one parallel group stay valid: each job's own
  policy applies to its own lines.

## 4. Log file and machine export

- The log file receives each line exactly once (the same forwarded line as
  the live stream when streaming, or the revealed line for capture/
  show-on-failure at reveal time).
- Machine export (`--events`, RUN-EVENTS-CONTRACT) receives structured
  records regardless of policy; suppressed output is still retrievable via
  `control output` from the bounded capture.

## 5. Show-on-failure semantics

- While the job is running: live output is suppressed.
- On failure: the buffered output is revealed exactly once (streamed with
  task attribution).
- On pass or cancellation: suppressed (still retrievable via capture).
- Superseded generations: revealed per the failure policy of the generation
  that actually reached terminal; superseded runs never double-reveal.

## 6. Parser and validation

- Invalid `output:` values are rejected loudly (exit 1 on `fzz check`).
- Legacy jobs without `output:` keep `inherit` — no behavior change.

## 7. Out of scope

- Per-command policies (the job is the unit).
- Color/formatting policies, desktop notification policies (TASK-0040 hooks
  compose for that).
