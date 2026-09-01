# Funzzy Finite-Job Timeout Contract (TASK-0138)

Status: accepted design. Implementation: TASK-0139. Proof: TASK-0140.
Amended after Kely's verification (evidence:
/Users/cristianoliveira/.agents/reports/26-08-26/task-0138-finite-job-timeout-contract-verification.md):
syntax grammar corrected to the shared parser, revision-identity encoding,
single ordering rule, evidence-surface filters, pi-watcher decoder location,
flush-dependency note. Round 2: job-wide sequential deadline (§3).

Evidence base (all cited symbols verified in-tree): `executor.rs` (`Executor`,
`advance`, `advance_task`, `Run`, `ActiveTask`, `Clock`, `POLL_INTERVAL`,
`shutdown_task`, `stop_after_failure`, `TaskState`, `TaskSnapshot`,
`record_task_snapshot`, `resolve_recoveries`), `process_owner.rs`
(`shutdown_policy`, `shutdown_all`), `watcher_state.rs` (`WatcherState`,
`tasks`), `awaiting.rs` (`TerminalReason`), `duration_recorder.rs`
(`record_success`/`record_failure`), `config.rs` (`GenerationHooks`,
legacy guards), `event_stream.rs` (NDJSON `TaskTerminal`), MANUAL-TRIGGER-CONTRACT.

## 1. Syntax, bounds, absence

`execution.timeout` optionally provides the default finite-job budget:

```yaml
execution:
  timeout: 10m
jobs:
  - name: lint                 # inherits 10m
    run: cargo clippy
  - name: integration
    timeout: 30m               # job override wins
    run: cargo test
```

The precedence is `jobs[].timeout` > `execution.timeout` > unbounded. The
execution default applies only to finite jobs; managed services remain
unbounded. `null`, zero, and sentinel strings (`inherit`, `unbounded`) are
invalid. The default is distinct from the control client's `--timeout`.


```yaml
jobs:
  - name: await-remote
    timeout: 30m
    run: ./scripts/await-remote.sh
```

- Property `timeout`, owner `Job` (option_catalog), string duration parsed
  with **the same grammar as `parse_debounce`**: `<number>` with optional
  `ms`/`s`/`m` suffix; a bare number means seconds; strictly positive.
  `1h`-style hours and composite `1h30m` are **NOT accepted today** —
  extending the parser is out of scope for TASK-0139 (recorded future
  extension, own decision); `30m` covers the motivating case.
- Bounds: strictly positive; `0s`/`0`, negative, and non-duration strings are
  actionable `InvalidConfigError` (like `service: yes` strictness). No maximum.
- Absence: no deadline — today's unbounded runtime. **No global default**
  (non-goal); absence is not a policy decision, it is "no timeout".
- Schema/help: `fzz config schema --section job` lists `timeout` (optional,
  duration string with ms/s/m units, default none); help: "Bound this finite
  job's execution; on elapse the job is terminated and fails the generation.
  Not a client await deadline."
- Preferred `jobs:` V2 only; `timeout` under legacy `tasks:` rejected at BOTH
  legacy parse sites (`rule_from` root-list guard and the grouped `has_tasks`
  guard in `parse_hash_format`), same shape as `recovery` (per amended
  manual-trigger contract precedent).

## 2. Deadline start and clock

- **Deadline starts at the job's first successful spawn** — the moment
  `advance_task` sets `task.started = Some(self.clock.now())` after
  `runner.spawn` succeeds. Not at schedule time (queue wait under load is not
  the job's fault), not at spawning attempt (spawn failures are already typed).
- Monotonic time only: the injected `Arc<dyn Clock>` (`Clock::now() -> Instant`);
  wall-clock time never drives deadlines. Tests inject a fake clock — no fixed
  sleeps (§9).
- Deadline = `started + timeout`, **per job**, covering the job's whole
  invocation: all its commands, including any deferred multi-command sequence.
  Re-spawns of the same job (service restart model) do not apply — §8.

## 3. Deterministic precedence

Evaluation happens inside the existing single-threaded `advance()` poll loop
(`POLL_INTERVAL` = 10 ms), which already makes every terminal condition
total-ordered. **Single ordering rule (applies once, consistently):** in
each `advance_task` iteration the timeout check runs BEFORE `try_wait` —
so a child that exited at `deadline − ε` but is reaped one poll later is a
**timeout** outcome (outcome indeterminism bounded by one poll interval;
accepted). Within one poll iteration the order is:

1. **Generation cancellation/supersession** — the existing
   `run.cancellation_requested()` guard at the top of `advance()` wins over
   everything: control `cancel`, replacement runs, reload reconcile.
2. **Watcher shutdown** (`process_owner::shutdown_all` path) — outside the
   loop, wins over job-level outcomes (existing behavior).
3. **Configured timeout** — deadline check runs in `advance()` before the
   per-task `try_wait` polling loop.
4. **Natural exit** — a child reaped by `try_wait`, which runs only after
   the deadline check in that same iteration.
5. **Fail-fast sibling failure** — `stop_after_failure` runs inside the
   per-task loop, after the deadline check.

**Sequential multi-command recheck (job-wide):** the deadline is the
ORIGINAL `ActiveTask.started + timeout` governing the job's whole
invocation (§2) — before each continuation spawn in a sequential run, the
ORIGINAL deadline is rechecked; `started` is never reset and no fresh
deadline is minted. A continuation that cannot meet the remaining budget
follows the same timeout precedence above (its spawn is already expired at
spawn-check time). This preserves the job-whole-invocation budget: a
two-command 30m job gets 30m total, not 60m.

Consequence (explicit): a timeout observed in the same pass as a sibling's
failure is a **timeout** for the timed-out job while the generation still
fail-fasts overall.

## 4. Timeout outcome typing

Distinct from command failure and from client-await timeout at every surface:

| Surface | Value |
|---|---|
| `TaskState` (task snapshot, NDJSON `TaskTerminal`) | new additive `TimedOut` (serde `"timedout"`); NOT `Failed` |
| Generation result | `Err("job 'NAME' timed out after DURATION and was terminated")` — generation fails |
| Await `TerminalReason` | stays **`failed`** (the generation failed); `Timeout` remains exclusively the client-await deadline (no conflation) |
| Human output | one error line: `Job 'NAME' timed out after DURATION; process group terminated (graceful, escalated on ignore)` |
| Retained evidence | captured output up to termination is retained/revealed per output policy (§6); **every surface that filters terminal evidence by `TaskState::Failed` today (`snapshot.rs`, `awaiting.rs`, `control.rs`) must also select `TimedOut`** so timeout evidence is retrievable identically |
| Exit code | process exit 1 (failed run) — unchanged mapping |

## 5. Termination, escalation, reap, duration

On deadline elapse with the child still un-reaped:

- Terminate the **whole process group** through the existing
  `shutdown_task` → `LoggedChild::shutdown(Signal, grace)` path
  (`process_owner::shutdown_policy()`: SIGTERM + bounded grace), escalating to
  SIGKILL when the group ignores the signal; then reap.
- Forwarding threads join at stream end, so output consumed before the kill
  is flushed (interacts with TASK-0141 batching: `ForwardHandles::join` is a
  flush boundary — pre-kill child-stream evidence reaches the log file
  deterministically). The **terminal timeout log line and any revealed
  output are conditional on the TASK-0141 final-flush fix** (explicit exit
  flush on real shutdown paths) — TASK-0139 ships after that fix lands.
- Duration accounting: task `duration_ms` = monotonic elapsed from `started`
  to terminal (deadline + shutdown completion; bounded by the grace period).
  Recorded via `record_failure` into duration history (a timeout is real
  failure-profile data — estimates must not treat it as success).

## 6. Output behavior

Evidence produced before/during shutdown stays bounded and attributable:
live forwarding keeps TASK-0028 attribution; bounded capture (TASK-0045 cap)
retains what arrived pre-kill; `show-on-failure` reveals it at terminal. No
new capture surface; nothing beyond the kill is readable by definition.

## 7. Interactions

- **Parallel groups:** timeout is per-job; surviving siblings keep their own
  deadlines. A timed-out job is a failure, so fail-fast (`stop_after_failure`)
  applies to siblings exactly as for command failures.
- **Recovery:** a timed-out job does **not** enter `pending_recoveries` —
  recovery targets command failures; re-running an unbounded command on a
  policy deadline would reintroduce unboundedness. Tradeoff recorded: users
  wanting retry-on-timeout compose it in the command (`timeout 30m cmd ||
  retry`), not here.
- **Hooks:** generation `failure` hooks fire (generation failed); `success`
  hooks never.
- **Reload / frozen revisions:** the timeout value is frozen into the task
  plan at schedule time; a reload changing `timeout` affects only generations
  scheduled under the new revision (same freeze discipline as concurrency,
  TASK-0090 AC7). **Revision identity:** `timeout` participates in
  `config_revision::encode_rule` (encoded like every other rule field, with
  absence a distinct canonical value from a present value) and the same
  change bumps `REVISION_SCHEMA_VERSION` — otherwise a timeout-only reload
  hashes as a no-op (same pattern as `trigger` in the 0135 amendment).
- **Duration history:** `record_failure` with measured duration (§5).
- **Manual trigger jobs:** orthogonal; `trigger: manual` + `timeout` is valid
  and is the primary use case (bounded observation of a blocking script).
- **`service: true`: rejected** — validation error, same family as
  `service`+`recovery`: a managed service is intentionally unbounded; a
  finite deadline contradicts the service contract. No silent reuse of finite
  terminal semantics.

## 8. Zero change and compatibility

Every config without `timeout:` behaves byte-identically (no deadline check
emitted). Legacy forms unchanged (§1). Declared compat surfaces unchanged
except the additive enum value below.

## 9. Control protocol / pi-watcher impact (recorded before TASK-0139)

- `TaskState` wire (`tasks[].state` in status/await snapshots, NDJSON
  `TaskTerminal.task.state`) gains additive `"timedout"`. No new methods, no
  new params; `capabilities` document text gains the enum value only.
- pi-watcher decoder lives in `pi-watcher/src/domain/capabilities.ts`
  (`WATCHER_TASK_STATES` union and `readTaskState`); it must add
  `"timedout"`. Additive, no breaking wire change (coordinate per repo
  AGENTS.md compatibility surface).
- Client `--timeout` (await deadline) is untouched — non-goal.

## 10. Test strategy (TASK-0139/0140)

Seams: injected `Arc<dyn Clock>` (fake clock advances past deadline while a
fake `ProcessRunner` child stays running — no real sleeps, no flakiness);
deadline arithmetic pure. Synchronization tests: timeout observed only via
clock advance; natural-exit-wins ordering; cancellation-beats-timeout
(token set before deadline elapse in same iteration).

Required cases: syntax/bounds validation (incl. `0s`, legacy rejection at
both sites); deadline-start-at-spawn (queued wait excluded); precedence
matrix of §3; typed outcome at every surface of §4 (TaskState, human line,
failures list, await reason `failed`, exit code 1); group sibling fail-fast;
no-recovery-on-timeout; hooks fire failure; frozen-revision deadline across
reload; duration-history failure record; service rejection; e2e
(`timeout: 100ms` sleeping child → terminated, generation failed, exit 1,
log contains the timeout line and pre-kill output).

## 11. Non-goals (restated)

Client await deadlines; provider/API polling deadlines; global default
timeout; remote approval or arbitrary cancellation policy.

## 12. Ambiguities flagged to lead (not resolved silently)

1. Whether generation-level await reason should eventually distinguish
   `timedout` (kept `failed` here to avoid conflation with client `Timeout`).
2. Whether recovery should be offerable post-timeout (rejected in §7;
   composition alternative recorded).
3. Whether `0s` should mean "fail immediately" (rejected: confusing; a job
   you never want to run is `trigger: manual` without invocation).
