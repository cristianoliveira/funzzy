# Funzzy advanced: parallel execution, control socket, and agent workflows

## 1. Parallel execution

### Named contiguous groups and barriers

Only **consecutive** jobs sharing one `parallel` name may overlap. A serial
job between them starts a new barrier — a reused group name never reconnects:

```yaml
on:
  change: "src/**"
execution:
  concurrency: 2
jobs:
  - name: lint
    parallel: checks
    run: cargo clippy
  - name: test
    parallel: checks
    run: cargo test
  - name: package
    run: cargo build      # serial: runs after both group members finish
```

- Commands inside one job stay strictly sequential.
- `execution.concurrency` caps simultaneously active tasks (default: available
  parallelism; `1` is valid and means one at a time inside the barrier).
- Ordering inside a group is intentionally unspecified; the summary lists
  every task with its group, keyed by identity, never by completion order.
- Filtering keeps the original topology: if only one group member matches, it
  runs alone — barriers stay valid.

### `--sequential` comparison for race-like failures

```sh
fzz run "@checks"            # configured (parallel) run
fzz run "@checks" --sequential   # effective concurrency one, same target
fzz ctl run "@checks" --sequential --wait --timeout 5m   # on a running watcher
```

Parallel fail + sequential pass is **`parallel-sensitive` evidence**, not
proof of a race root cause (SEQUENTIAL-OVERRIDE-CONTRACT §8). Never
auto-retry side-effecting commands sequentially.

### Failures and restart

- Default: a failed task does not stop siblings or later stages; the run
  fails with combined results.
- `--fail-fast`: cancels active siblings and skips queued/later work on the
  first failure.
- `--on-busy restart` / `--restart`: a newer event cancels and reaps all
  active tasks across every group, then starts the newest generation.

### Workload tradeoffs

Parallelism helps when independent tasks dominate a batch — latency
approaches the slowest task, not the sum. It does not help CPU-bound tasks on
few cores or tasks competing for one resource (database, port, lock file).
Start at `concurrency: 2` and measure. Measure with:

```sh
time fzz run "@bench"                  # serial (concurrency: 1)
time fzz run "@bench" --sequential
```

## 2. Control socket

`control` is canonical; `ctl` is its visible alias (identical behavior).

### Socket precedence

1. `fzz ctl --socket PATH`
2. `--control-socket PATH` (global)
3. `on.socket` from the selected config

### Capabilities first

```sh
fzz ctl capabilities --format json
```

Advertises protocol/schema versions, methods, limits, features (including
`sequentialOverride`). Gate on facts, never on package versions; a legacy
server without `capabilities` reports a `legacy` profile.

### Method summary

| Method | Purpose | Key output |
| --- | --- | --- |
| `status` | current state | generation, state, failures |
| `list` | remote targets | names + commands |
| `run TARGET [--wait]` | schedule exact generation | runId |
| `emit PATH [--wait]` | synthetic change through native routing | outcome, matched, runId |
| `await` | atomic wait (after/exact) | terminalReason, snapshot, freshness |
| `output` | bounded retained output | content + truncation bounds |
| `cancel` | exact-generation cancel | cancelled, generation |
| `capabilities` | negotiation facts | methods, limits, features |

`--wait` requires `--timeout`. Exit codes: 0 success/no-op, 1 failed/
superseded/timeout/restarted, 2 usage. `--format toon|json|human` selects
structured output (TOON default for agents, JSON interoperability).

### Freshness

`freshness: current` means the snapshot is exactly the requested generation;
`stale` means a newer batch exists. Never reconstruct freshness by polling.

### Recovery and control-socket boundaries

A control client can observe recovery phase events and await the final result,
but it cannot approve a recovery in this MVP. Approval is local to the
foreground watcher TTY. Use an explicit policy for headless runs:

```sh
fzz run --recovery-policy skip "@quick"
fzz watch --recovery-policy skip
```

With `prompt`, the watcher waits for one attached TTY answer after original
jobs quiesce. Only `y` or `yes` runs the exact declared recovery commands; the
commands run once and the original job is verified once. Empty input, `n`,
invalid input, EOF, no TTY, cancellation, and supersession preserve failure.
`--recovery-policy` overrides `execution.recovery_policy` for that invocation;
`CI=true` is not inferred as approval.

## 3. Agent edit-feedback loop

The compact loop (proven end-to-end, AGENT-FEEDBACK-CONTRACT §9):

```sh
# 1. Negotiate capabilities (legacy fallback detection).
fzz ctl capabilities --format toon

# 2. Trigger and await the exact generation.
fzz ctl emit <path> --format toon                # -> runId
fzz ctl await --generation <runId> --timeout 5m --format toon
#    -> terminalReason + snapshot + freshness

# 3. On failure: retrieve task-attributed evidence, fix, re-await.
fzz ctl output --generation <runId> --tail 80 --format toon
fzz ctl await --generation <newRunId> --timeout 5m --format toon

# 4. Cancel obsolete work (exact generation, compare-and-act).
fzz ctl cancel --generation <runId> --wait --timeout 15s
```

- A successful loop needs at most one emit + one await; failures add one
  `output` call.
- `terminalReason` distinguishes passed/failed/cancelled/superseded/timeout/
  restarted/disconnected — never guess from stdout.
- On watcher restart the instance token changes; treat it as explicit
  invalidation, never a false terminal result.
- Legacy servers: capability-gated fallback; a `sequentialOverride`-less
  server rejects `--sequential` before scheduling (never silent parallel).

## 4. Duration estimates

Local history lives under the XDG state dir, keyed by stable execution
signature (plan content, not YAML spelling). See DURATION-ESTIMATES-GUIDE.

- Eligibility: exact target runs with history.
- Confidence: None/Low/Medium/High from sample count; estimates are advice,
  never an ETA.
- Timeout precedence: explicit `--timeout` wins; a run slower than its
  history is reported honestly.
- Invalidation: changing job content/topology changes the signature; a
  `tasks:` → `jobs:` rename alone does not.
- Privacy/reset: delete the state file to reset history.

## 5. pi-watcher

pi-watcher is a **Pi extension**, not Funzzy: it consumes the control socket
and negotiates capabilities. Funzzy execution truth comes from the watcher;
Pi projection (session activity, checkpoints) is the extension's own layer.
Legacy fallback is capability-gated — a server without `capabilities` gets
the `legacy` profile, never assumed facts.

## 6. Troubleshooting

| Symptom | Cause → action |
| --- | --- |
| No match | path matches no change glob → `fzz explain PATH` |
| Ignored path | config `ignore` or gitignore won → explain names the source |
| Ambiguous target | substring matches several → `fzz list` shows exact names |
| Socket unavailable/stale | watcher down or restarted → check `capabilities` token, restart watcher |
| Superseded generation | newer batch replaced it → await the newer runId |
| Truncated output | retention cap → `output --tail N` / `--full`, bounds reported |
| Process cleanup | cancelled children must be reaped → verify `cancel --wait` terminal |
| Config reload | see §6.1 — four distinct behaviors, never guess from a single symptom |
| Corrupt history | state file damaged → delete to reset, or rely on confidence=None |
| Feedback loop | watcher noise from generated files → `ignore` or `respect_gitignore: true` |

### 6.1 Config reload — four distinct behaviors

Editing `.watch.yaml` while a watcher runs is **not** one behavior. Four
outcomes exist, and each is observable (CONFIG-RELOAD-CONTRACT):

| Behavior | When | What you see | Process identity |
| --- | --- | --- | --- |
| **Valid hot reload** | candidate passes all four gates and differs semantically | `Config change is valid; hot-reloading to revision N.` — jobs/roots/policy swap in-process | same PID, same instance token, revisions increment monotonically, subscribers stay connected |
| **No-op save** | formatting/comment-only rewrite, or semantically identical | `Config save has no semantic change; nothing to reload.` | unchanged revision, no churn |
| **Invalid fatal exit** | any gate fails (broken YAML, schema/semantic error, occupied new socket, unwatchable root) | `Fatal configuration error; terminating watcher.` + gate/reason + nonzero exit; `configInvalid` published to subscribers before the socket closes | process exits; descendants/services reaped; no stale config kept silently |
| **True external restart** | you (or a supervisor) start a new watcher process | new process start, new instance token in `capabilities` | PID and instance token change; generations restart at 1 |

Additional rules:

- **Managed services** (`service: true`) are reconciled at the commit
  boundary: an unchanged service stays owned; a changed/removed one is
  gracefully stopped and replaced without overlapping same-name instances.
  Services with `readiness` settle their generation after the probe succeeds;
  services without it retain legacy generation-owned, unbounded behavior. A
  config edit never leaks a second server on the same port.
- **A config save never kills an active finite run**: the running generation
  completes under its original revision; later events route under the new
  one (output references carry each generation's frozen revision).
- **A deleted config is fatal** (`config unreadable after change`): the
  watcher cannot run without a config. A transient delete+recreate inside
  the debounce window settles on the final candidate without spinning.
- **Partial editor writes** settle inside the debounce/stable-read window: a
  truncated write followed by the valid final content reloads once; content
  left invalid after the window is a fatal exit.
- **Rapid saves** debounce to the final candidate: one revision commit, never
  a mixed or intermediate revision.
- **Distinguish reload from restart by fact, not feel**: compare
  `capabilities.instance.token` (or the PID) across the change. A preserved
  token means the watcher hot-reloaded; a changed token means a new process.

## 7. Machine-readable output

- Control: `--format toon|json|human` (one document, deterministic).
- Run events: `--events FILE` appends schema-versioned NDJSON
  (`started`/`tick`/`task_terminal`/`finished`/`cancelled`) with run/task/group
  identity (RUN-EVENTS-CONTRACT). Managed-service lifecycle is intentionally
  omitted from this MVP stream; inspect its secondary `name,state` view through
  control status/await/subscription.
- Never teach polling-based freshness reconstruction or deprecated
  compatibility paths in these recipes.

## 8. Integration-agnostic command observation

Any external result — a remote CI run, a deploy, a long query — can be
observed as one finite, explicitly requested job, without coupling Funzzy to
any provider. A blocking user script represents the external system; Funzzy
observes process lifetime and exit status. This is a documented recipe over
existing capabilities, not a provider API (TASK-0137).

### 8.1 The recipe

```yaml
on:
  socket: .tmp/funzzy/control.sock

execution:
  timeout: 10m          # default for finite jobs
jobs:
  - name: lint
    run: cargo clippy    # inherits 10m
  - name: await-remote
    trigger: manual
    timeout: 30m          # job override wins
    run: ./scripts/await-remote.sh
```

`trigger: manual` (MANUAL-TRIGGER-CONTRACT) means explicit invocation only:
the job never runs at watcher init, never matches filesystem events — even
when a root `on.change` would select every other job — and never inherits
root patterns. It starts only through explicit selection. `timeout:`
(optionally) bounds the job's execution (FINITE-JOB-TIMEOUT-CONTRACT):
on elapse the whole process tree is terminated and the generation fails
with the typed `timedout` task state — bounded observation without the
script implementing its own deadline.

```sh
# Foreground composition (local, one-shot process):
git push && fzz run await-remote

# Running-watcher composition (the watcher owns the work):
git push && fzz ctl run await-remote --wait --timeout 31m --format toon
```

`fzz run` exits with the job's combined outcome, so `&&` chains stop on
failure and never start the target after a failed predecessor. `fzz ctl run`
schedules an exact generation on the running watcher; `--wait` returns its
terminal observation in one round trip.

### 8.2 The script boundary

- **The script owns** authentication, provider polling, correlation,
  retries, and every provider semantic. It blocks while the external work is
  pending and exits once: `0` for success, non-zero for failure.
- **Funzzy owns** configured command execution and observation: exact
  generation identity, alive-means-running status, bounded stdout/stderr
  retention, process-group cancellation, and reload isolation.
- **Opaque exit caveat:** a non-zero exit cannot distinguish remote failure
  from script/API failure. The script's stdout/stderr must carry actionable
  evidence (`fzz control output --generation N` retrieves it, bounded).

### 8.3 Synchronous and asynchronous flows

- **Synchronous:** `ctl run TARGET --wait --timeout D` — one round trip,
  terminal observation included. Use `--format toon|json` for
  machine-readable composition (§7).
- **Asynchronous:** take the returned generation (`runId`), then follow up
  with exact-generation APIs — `control await --generation N --timeout D`,
  `control output --generation N [--task T]`, and
  `control cancel --generation N --wait` (a stale cancel is a safe no-op;
  later generations are never affected).

Control `--timeout` bounds **waiting only**; it never terminates the
observed process. The job's own lifetime is bounded separately by
`jobs[].timeout` (§8.5) — the two deadlines are independent surfaces.

### 8.4 Choosing a finite job or a readiness-enabled service

A legacy managed service without `readiness` is the wrong primitive for one
finite observation: it is generation-owned, unbounded, and keeps the
generation running while alive. It is reaped on supersession and restarted
only by re-inclusion (SERVICE-LIFECYCLE-CONTRACT).

Use a readiness-enabled service when the goal is a long-lived process whose
startup health should produce a truthful terminal generation result. Its
readiness probe must pass before the generation settles; the service then
remains in the worker-owned pool and status reports `services: [{name,state}]`.
A blocking observation still needs exactly one invocation with one terminal
result, so use `trigger: manual` on a normal finite job.

### 8.5 Bounded observation: `jobs[].timeout` vs control `--timeout`

Two independent deadlines — never conflate them:

- **`execution.timeout: <duration>`** sets the default child lifetime for finite jobs; **`jobs[].timeout`** overrides it. Services remain unbounded by this finite-job timeout; a readiness service's `readiness.timeout` bounds health probing, not service uptime. Both use (`30m`,
  `500ms`, `90s`; a bare number means seconds). It starts at the job's
  first successful spawn, covers the job's whole invocation (including
  multi-command sequences), and on elapse terminates the complete process
  group — graceful SIGTERM, escalating to SIGKILL, then reap. The job
  records the typed `timedout` state, the generation fails, pre-kill output
  stays retrievable (`control output --generation N`), duration history
  records a failure, and recovery is never offered. Exact user cancellation,
  generation supersession, and watcher shutdown all outrank the deadline.
- **Control `--timeout`** owns only the **caller's wait**: an await that
  expires reports `timeout` and performs no cancellation whatsoever.

A config reload that changes `timeout` affects only generations scheduled
under the new revision; running generations keep their frozen deadline and
a stale timer never touches replacement work. `timeout` is rejected on
`service: true` jobs (a managed service is intentionally unbounded).

Proven black-box in `tests/finite_job_timeouts.rs`.

### 8.6 Proof

The recipe is proven black-box in `tests/manual_observation_recipe.rs`
(manual silence, alive→running, exit mapping, exact-generation isolation,
TOON, foreground composition) and in `tests/finite_job_timeouts.rs`
(deadline elapse terminates the process tree with the typed `timedout`
outcome, natural outcomes unchanged, cancellation precedence, reload
freeze, and local/control agreement).
