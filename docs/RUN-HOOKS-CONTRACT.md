# Funzzy Run and Watcher Hooks Contract

## 1. Scope and ubiquitous language

Funzzy has two different terminal boundaries. Their hooks must not be
confused:

| Hook | Boundary | Cardinality | Correlation |
|---|---|---|---|
| `hooks.success` | one generation passes | once per passing generation | generation ID |
| `hooks.failure` | one generation fails | once per failing generation | generation ID |
| `hooks.close` | one ready watcher session closes | at most once per process/session | watcher instance + config revision; **no generation ID** |

- **Generation terminal hooks** (`success`/`failure`) observe workflow
  outcomes. They may run many times in one watcher session.
- **Watcher-session terminal hook** (`close`) observes the watcher lifecycle.
  It is not “run another command after every workflow.”
- No task/group-level hooks in this revision.
- Every hook is an optional, generic, **finite shell command**. Composition is
  the user's choice (for example, call a notification or cleanup script),
  never a built-in platform integration.

## 2. Configuration shape and validation

Preferred V2 form (production-parser validated):

```yaml
hooks:
  success: ./scripts/on-success
  failure: ./scripts/on-failure
  close: ./scripts/on-close

jobs:
  - name: test
    run: cargo test
    change: "**/*"
```

- `hooks.close` is a sibling of `hooks.success` and `hooks.failure`; `on_close` is rejected as an unknown property.
- Preferred `jobs:` configuration accepts hooks only under `hooks:`. Grouped legacy `tasks:` retains its historical `on:` hook placement as a compatibility input; a legacy root task list cannot declare hooks. Unsupported root/mixed shapes remain errors; migration does not change hook semantics.
- `hooks.success` and `hooks.close` values are one non-empty shell-command
  string. `hooks.failure` accepts either that scalar (immediate behavior) or
  the failure-only object `{ run: <non-empty string>, settle: <positive
  bounded duration> }`. Lists, null, empty strings, unknown properties, and
  objects on `success`/`close` are actionable config errors. A failure object
  must contain exactly `run` and `settle`; `settle` must use the normal
  duration syntax and be greater than zero and no greater than the configured
  maximum (default 24h).
- The command must be finite. A service/daemon belongs in a `service: true`
  job, not a close hook.
- Parser allowlist, JSON Schema, canonical option catalog, `fzz check`, and generated init comments must agree that `close` is an optional `hooks` string.

## 3. Generation terminal hooks (unchanged TASK-0040 behavior)

- `hooks.success` runs exactly once when the generation passes (every task passed, no cancellation).
- `hooks.failure` runs exactly once when the generation fails (any task failed).
- A **superseded** generation (replaced by a newer run) runs neither hook: it
  was never a completed outcome, only displaced.
- A **cancelled** generation runs neither hook: cancellation is an explicit
  interruption, not a result.
- Ordering: generation hooks run after that generation's own tasks reach
  terminal and before the next generation starts.
- Hook failure does **not** change the generation outcome or exit code. It is
  visible in verbose/structured output.
- Generation hooks expand `{{filepath}}`/`{{paths}}`, carry generation
  identity, and produce generation-correlated hook events.

## 4. Settled failure hooks

The scalar `hooks.failure: COMMAND` remains immediate and is unchanged. The
object form is deliberately failure-only; it does not alter success hooks or
introduce a shared hook policy.

For `hooks.failure: {run: COMMAND, settle: DURATION}`, a failed generation is
published as terminal immediately, then a settle timer starts at the instant
that terminal failure is committed. The timer is owned by that generation and
its immutable committed configuration revision. Settlement means that this
failure remains the latest accepted generation outcome for the entire duration;
knowledge of editor or agent activity is not involved.

Scheduling a newer generation is the supersession boundary: once accepted by
the watcher (including a control-triggered run), it invalidates the pending
settled hook before replacement work starts. A newer failure gets its own
snapshot and timer. A newer pass, cancellation, or supersession leaves no
failure hook pending. The wait is asynchronous and never blocks scheduling,
starting, terminal publication, or control responses for newer work.

Timer expiry and acceptance of a newer generation are serialized by the
watcher's generation coordinator. If acceptance wins, the pending hook is
cancelled and reaped and does not start. If expiry wins, the command is
atomically claimed and starts once; a generation accepted afterward cancels
only the already-running command using the normal process-group cancellation
and grace/reaping policy. A command that has begun cannot have external side
effects recalled. In either order, no settled hook runs twice and no stale
pending timer remains.

The settled command receives the same generation correlation, template
expansion, working directory, environment, output handling, and hook-failure
semantics as an immediate failure hook. Its command and configuration revision
are snapshots from the failed generation, so reloads do not rewrite a pending
or running hook. A valid reload may affect later generations; malformed
reloads do not. Hook failure (spawn error, nonzero exit, signal, or timeout)
never changes the generation result or schedules another hook.

Finite `run` commands use immediate failure hooks only and never start a settle
timer. In watched mode, each accepted generation follows the rules above;
control `run`/await observes terminal publication without waiting for the
settle timer. Reload, cancellation, supersession, and shutdown cancel pending
settled timers. Shutdown first quiesces scheduling, then cancels and reaps
pending/running settled hooks alongside other owned work; it does not run a
new settled hook after the close gate. A failure committed before shutdown may
run only if its timer was already claimed before quiescence, and is then reaped
by shutdown. These rules apply equally to normal return, control await,
reload, and graceful shutdown.

## 5. Watcher close lifecycle and exact ordering

A close hook is eligible only after the watcher reaches **ready**: filesystem
watches are registered and any configured control socket is accepting
requests. Startup failure before this point cannot run it.

Every graceful close path uses one shared, atomically claimed state machine:

```text
running
  -> closing (first caller atomically wins; first reason/exit code freezes)
  -> stop accepting filesystem events and control scheduling
  -> publish terminal watcher lifecycle reason to existing subscribers (best effort)
  -> cancel and reap active generations, generation hooks, and services
  -> close/unlink control socket and other watcher resources
  -> run latest committed hooks.close at most once (when configured)
  -> report close-hook outcome
  -> exited (with frozen original reason/exit code)
```

Normative ordering:

1. **Claim once.** Signal handler, fatal-config path, normal return, and any
   concurrent internal shutdown caller race through one compare-and-set gate.
   Only the winner can advance lifecycle; losers may request faster
   cancellation but can never run another hook.
2. **Quiesce first.** Stop accepting/scheduling filesystem and synthetic
   control events. No new generation can be created after `closing` begins.
3. **Reap owned work.** Active jobs, services, and in-flight generation hooks
   receive the existing coordinated process-owner shutdown policy and are
   fully reaped. Close hook never overlaps owned workflow work.
4. **Retire resources.** Existing subscribers receive the original terminal
   watcher reason best-effort, then control socket closes/unlinks. No client
   can schedule work while the close hook runs.
5. **Run close hook.** Snapshot the latest successfully committed `hooks.close`
   and execute it once. Absence is a successful no-op.
6. **Exit unchanged.** Report hook result, then exit with original frozen
   watcher reason/code.

The watcher is already quiescent before step 5. Files the close hook writes
are not observed and cannot schedule another generation.

## 5. Eligible and ineligible exits

`hooks.close` runs once for a **ready watcher** on:

- graceful SIGINT (original exit code `130`);
- graceful SIGTERM (original exit code `143`);
- fatal runtime config shutdown after an invalid/unpreparable reload
  (original nonzero config exit reason/code);
- any normal return from `fzz`/`fzz watch` (original exit `0`);
- internal graceful watcher shutdown using the same close coordinator.

It cannot run on:

- SIGKILL, power loss, process abort, or runtime crash that bypasses graceful
  cleanup;
- startup/config failure before readiness;
- a second signal or duplicate shutdown request (the first close owns it).

Finite/non-watcher commands never run `hooks.close`, even when they read a config:

- `fzz run`, `check`, `list`, `explain`, `exec`;
- `fzz config schema|example`, `init`, `migrate`;
- all `fzz control|ctl` clients.

A control client disconnecting or a generation ending is not watcher close.

## 6. Reload and revision semantics

- Runtime configuration owns `hooks.close` with the rest of the immutable
  committed revision (CONFIG-RELOAD-CONTRACT).
- At close-gate claim, snapshot the **latest successfully committed** value.
  A valid reload may add, replace, or remove it; that committed value (or
  absence) is authoritative for eventual close.
- A malformed/unpreparable candidate never commits and never replaces the
  last valid hook. If that candidate causes fatal shutdown, the last valid
  committed `hooks.close` runs.
- Formatting-only reload remains a no-op: revision and hook identity do not
  churn.
- Hook execution uses the snapshot taken at close claim; files changed during
  close cannot reload or replace it.

## 7. Working directory, environment, and templates

- Close hook runs from the immutable watcher workspace root, not the shell's
  later cwd and not a job-specific `cwd`.
- It inherits the watcher process environment. No synthetic secrets,
  filepath, changed-path batch, task name, run ID, or generation ID are
  injected.
- `{{filepath}}`, `{{absolute_path}}`, `{{relative_filepath}}`,
  `{{relative_path}}`, and `{{paths}}` are trigger-bound and therefore invalid
  in `hooks.close`. `fzz check` rejects them with an actionable “close has no
  trigger path” error. Unknown templates follow existing hook/template error
  policy; they are never silently replaced with empty text.
- Generation hooks retain their existing trigger-template support (§3).

## 8. Failure, timeout, signals, and exit codes

- Close-hook success is visible, but produces no new workflow outcome.
- Spawn failure, nonzero exit, signal death, or timeout is visible on stderr,
  verbose output, and configured log output. It never replaces the original
  watcher close reason/code.
- Hook runtime is bounded by the existing process cancellation grace
  (`FUNZZY_CANCEL_GRACE_MS`, default 5000 ms). This reuses one explicit
  process-lifecycle policy rather than adding a second hidden timeout.
- On deadline, the hook process group receives configured cancellation signal
  (`FUNZZY_CANCEL_SIGNAL`, default SIGTERM); process ownership waits its
  configured grace and escalates to SIGKILL, then reaps every descendant.
- A second SIGINT/SIGTERM while hook is running requests immediate coordinated
  cancellation/reaping. It cannot enter the hook again, and the **first**
  shutdown reason/code still wins deterministically.
- The entire graceful close path is therefore bounded; no hook can keep the
  watcher process alive forever or orphan descendants.

## 9. Recursion and feedback

Generation hooks may change watched files and intentionally trigger a newer
generation; those loops stay observable. Close hook differs:

- watcher scheduling is disabled before it runs;
- changed files remain on disk but cannot feed back into this closing session;
- spawning another detached watcher/process is user-owned behavior and not
  managed as a close hook descendant after successful hook completion;
- direct known feedback-loop declarations remain validation errors under the
  existing hook policy.

## 10. Correlation, output, and control socket

- Close-hook diagnostics use explicit `close hook` vocabulary and carry
  watcher instance identity + committed config revision when those are
  available. They never invent generation `0`, reuse the last generation, or
  allocate a fake generation.
- stdout/stderr use the normal process runner and configured log mirroring.
  Close-hook output is not placed in generation-keyed retained output and is
  not affected by task output policy.
- Existing control subscribers receive the **watcher terminal lifecycle
  reason**, not a fake run outcome. Socket closes before hook execution, so
  clients observe terminal/disconnected state and can never schedule work
  during cleanup.
- Run-event NDJSON remains generation-oriented: `hooks.close` emits no generation
  `hook` record. A future watcher-lifecycle event format must be separately
  versioned rather than smuggling close data into a generation schema.

## 11. Compatibility surfaces and proof

TASK-0101/0102 must update and prove these together:

- parser allowlist and errors for grouped preferred/legacy shapes;
- canonical option catalog, JSON Schema, `fzz check`, comprehensive init
  template/golden snapshot, and config examples;
- README, USAGE/advanced configuration, and this contract;
- valid/invalid hot reload and revision snapshots;
- coordinated SIGINT/SIGTERM, fatal-config, service/job/generation-hook
  shutdown paths and descendant reaping;
- control subscriber terminal ordering and socket unlink;
- both binary aliases (`funzzy`, `fzz`), normal and repeated signals;
- GitHub issue #234 completion evidence (tests + docs + release/PR reference).

Minimum black-box matrix: configured/absent hook × normal return/SIGINT/SIGTERM/
fatal invalid reload; active finite job; active managed service; hook success/
nonzero/timeout; second signal; latest valid reload vs malformed candidate;
finite commands proving zero close-hook execution.

## 12. Out of scope

- Desktop/browser notifications (compose a command).
- Task/group-level hooks or DAG-style hook chaining.
- Running close after SIGKILL, startup failure, or crash.
- Giving close hook generation identity, retained-generation output, changed
  paths, or a synthetic workflow outcome.
- Allowing close hook to change the watcher exit reason/code.
