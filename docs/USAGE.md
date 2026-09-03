# Funzzy V2 usage: getting started and daily workflows

> V2 status: current `develop` is an unreleased **2.0.0** with intentional CLI
> breaks (RELEASE-BOUNDARY). Both binary names work: `funzzy` and its short
> alias `fzz` — examples use `fzz` consistently.

## 1. Getting started (five steps)

The shortest accurate path from installation to a running workflow.

### 1.1 Create a config

```bash
fzz init                       # writes the comprehensive commented starter
fzz init --template minimal   # or: minimal | parallel | agent
```

`fzz init` is **create-only**: it refuses an existing `.watch.yaml`
(exit 1, bytes untouched) and never overwrites or migrates. To copy a
starter into a custom path, pipe the side-effect-free export:

```bash
fzz config example minimal      # prints runnable .watch.yaml bytes (byte-identical to init --template minimal)
fzz config schema               # prints the JSON Schema for the jobs: format
fzz config schema --section on  # one bounded section + hint for the full schema
```

To rewrite a legacy config in place, use the explicit transform:

```bash
fzz migrate                     # .watch.yaml (or -c PATH): legacy -> jobs:, atomic, idempotent
```

The preferred V2 shape is an ordered `jobs:` list:

```yaml
on:
  change: "**/*"

jobs:
  - name: build
    run: "cargo build"
```

Declaration order is semantic: consecutive jobs sharing the same `parallel`
group name may overlap; everything else runs in order.

### 1.2 Validate before watching

```bash
fzz check           # loads the same parser/validator the watcher uses
```

`check` reports schema errors, invalid globs/durations/concurrency, and path
existence. Exit 0 when valid.

### 1.3 Run once, locally

```bash
fzz run build       # run the exact target once; no watcher, no socket
fzz run "@quick"    # target can be a name, @tag, or unambiguous substring
```

`run` exits with the combined outcome (0 all pass, 1 any fail).

### Readiness-enabled services

A service without `readiness` remains a legacy generation-owned service: it is
unbounded and keeps its generation running while alive. Add an explicit
readiness command when the watcher should settle the generation after health
is proven:

```yaml
jobs:
  - name: api
    service: true
    run: cargo run -- --port 8080
    readiness:
      run: curl --fail http://127.0.0.1:8080/health
      timeout: 30s
      interval: 500ms
```

A successful readiness probe transfers `api` to the worker-owned pool. The
exact generation becomes `passed`, while control status reports the separate
secondary view `services: [{name: api, state: ready}]`. A later service
restart or failure does not rewrite that generation result. A failed probe or
timeout fails the generation and reaps the service. Readiness configuration
is strict and frozen when the generation is scheduled.

### Current-run job durations

Finite runs render one declaration-ordered `JOB / RESULT / DURATION` row for
each configured job. Per-job duration is the executor's monotonic elapsed
measurement, valid whether jobs are serial or parallel. The generation
`Duration:` remains separate wall-clock time and is never calculated by adding
job rows.

Started cancellations retain a partial duration; skipped or never-started jobs
render `-` and structured snapshots/events expose `durationMs: null`. A recovered job appears once with its final state and a duration through final
verification. Readiness-enabled services record duration through readiness
promotion; their later uptime is pool state. Legacy live services have no
finite duration while alive. For exact integer milliseconds, inspect
`tasks[].durationMs` in a control snapshot or a `task_terminal` NDJSON event.

### 1.4 Inspect before executing

```bash
fzz list            # configured jobs (targets)
fzz explain src/lib.rs   # which jobs match/ignore a path + the filtered plan
```

### 1.5 Watch

```bash
fzz                 # zero-argument: configured watch (hero path)
fzz watch "@quick"  # watch only matching targets
fzz watch --exclude docs
fzz watch "@quick" --exclude lint --no-services
```

Watch filters are invocation-only: `TARGET` is selected first, then each
repeatable `--exclude TARGET` removes an exact job name, every job carrying an
`@tag`, or one unambiguous name substring. `--no-services` is the shortcut for
excluding every `service: true` job, including readiness-enabled services.
Selectors are validated before watcher roots, service processes, or readiness
probes start. The filtered plan keeps declaration order and parallel-group
barriers. These options affect only `fzz`/`fzz watch`; they do not change the
YAML file, `fzz run`, or control-socket requests, and the same invocation
policy is reapplied to valid configuration reloads.

This is the whole loop: config → check → run → watch.

## 2. Daily workflow decision table

| Goal | Command | Watcher? | Side effects |
| --- | --- | --- | --- |
| Watch + run on change | `fzz` / `fzz watch [TARGET]` | yes | runs tasks, may open socket |
| Run once, finite | `fzz run TARGET` | no | runs tasks, exits |
| Validate config | `fzz check [-c PATH]` | no | none |
| List targets | `fzz list` | no | none |
| Explain a path | `fzz explain PATH` | no | none |
| Ad-hoc over stdin | `fzz exec -- PROGRAM ARG...` | no | runs PROGRAM per stdin path |
| Control running watcher | `fzz control status\|list\|run\|emit\|await\|cancel\|output\|capabilities` | no | talks to the socket |
| Init a starter config | `fzz init [--template P]` | no | writes `.watch.yaml` (create-only, refuses existing) |
| Migrate a legacy config | `fzz migrate [-c PATH]` | no | atomic in-place rewrite (idempotent) |
| Config discovery | `fzz config schema\|example` | no | none (never reads project config) |

**Busy policy**: `--on-busy wait|restart` (default `wait`); `--restart`
cancels and reaps active work on a newer event. **Fail fast**: `--fail-fast`
stops at the first failing task. **Logging**: `--log-file FILE` mirrors all
output; **events**: `--events FILE` appends NDJSON run events.

**Exit codes**: 0 success/no-op, 1 workflow/operational failure, 2 usage. `--recovery-policy prompt|skip` overrides the configured policy for `fzz run` and watch sessions; it does not edit the config.

## 3. Configuration guide

The preferred grouped shape (JOBS-CONFIG-CONTRACT):

```yaml
on:
  change: "src/**"        # common change globs for all jobs
  ignore: "**/*.log"      # common ignore globs
  socket: .tmp/funzzy/control.sock   # enable the control surface
  debounce: 500ms         # filesystem batch window (default 1s)
  watch_backend: auto     # native | poll | auto (native first, poll fallback)
  respect_gitignore: true # respect workspace .gitignore (default false)

execution:
  concurrency: 2          # scheduler bound (default: available parallelism)
  output: show-on-failure # default job output policy
  recovery_policy: prompt # prompt | skip; default prompt
  timeout: 10m           # default finite-job timeout

hooks:
  success: echo "checks passed"
  failure: echo "checks failed"
  close: echo "watcher stopped"

jobs:
  - name: lint
    parallel: checks      # contiguous members may overlap
    run: cargo clippy
    cwd: packages/core    # per-job working directory
    env: { MODE: prod }   # per-job environment
    change: "src/**"      # per-job triggers
    ignore: "target/**"   # per-job ignores (strongest precedence)
    run_on_init: true     # run when the watcher starts
    trigger: manual       # explicit run only: never matches events or init
                             # (see ADVANCED-GUIDE §8)
    timeout: 30m          # bound execution; elapse terminates the job and
                             # fails the generation (not a client wait bound)

  - name: api
    service: true
    run: cargo run -- --port 8080
    readiness:
      run: curl --fail http://127.0.0.1:8080/health
      timeout: 30s
      interval: 500ms

  - name: format-check
    timeout: 30m          # job override wins
    run: cargo fmt --all -- --check
    recovery: cargo fmt --all # offered only after an explicit failure approval
```

- **Matching**: a job runs when a change glob matches and no ignore glob wins.
  Explicit config `ignore` beats gitignore; gitignore applies only with
  `respect_gitignore: true` (GITIGNORE-CONTRACT).
- **Manual trigger**: `trigger: manual` removes a job from every automatic
  surface — no init run, no filesystem matching, no root `on.change`
  inheritance; it starts only via `fzz run TARGET` or `fzz ctl run TARGET`
  (MANUAL-TRIGGER-CONTRACT, ADVANCED-GUIDE §8).
- **Execution timeout**: `execution.timeout: <duration>` sets the default for finite jobs; a job's `timeout:` overrides it. Services remain unbounded by this finite-job timeout; `readiness.timeout` only bounds startup health probing. `timeout: <duration>` (`30m`, `90s`, `500ms`; a
  bare number means seconds) bounds the job's whole invocation. On elapse
  the complete process group is terminated, the job records the typed
  `timedout` state, the generation fails, and pre-kill output stays
  retrievable. It is independent from control `--timeout`, which bounds only
  the caller's wait (FINITE-JOB-TIMEOUT-CONTRACT, ADVANCED-GUIDE §8.5).
- **Templates**: `{{filepath}}` (trigger path, backward compatible),
  `{{paths}}` (whole batch, shell-escaped), `{{relative_filepath}}`.
- **Parallel groups**: only *consecutive* jobs sharing one `parallel` name may
  overlap; reused names across a serial job start a new barrier. Order inside
  a group is unspecified (PARALLEL-EXECUTION-CONTRACT).
- **Hooks**: `hooks.success` runs after each passing generation. Scalar `hooks.failure` runs immediately; object-form `hooks.failure.run` waits for `settle` only while that failed watcher generation remains latest. Neither changes the result. `hooks.close` runs once, only when a ready watcher shuts down gracefully after active jobs/services are reaped. Finite commands do not run settled or close hooks (RUN-HOOKS-CONTRACT).
- **Recovery**: `jobs[].recovery` is an ordered scalar or command list. A failed finite job is recoverable only under `execution.recovery_policy: prompt`, after an attached TTY answers `y`/`yes`; recovery commands run once, then the original job is verified once. `n`, EOF, invalid input, no TTY, and `skip` preserve the original failure without spawning recovery.
- **Legacy input**: root task lists and grouped `tasks:` remain accepted and
  are rewritten deterministically with `fzz migrate`.

## 4. Approved recovery workflow

Use recovery only for a bounded, known-safe mutation. Configuration declares
what may be offered; it never authorizes execution by itself:

```yaml
execution:
  recovery_policy: prompt

jobs:
  - name: format-check @quick
    run: cargo fmt --all -- --check
    recovery: cargo fmt --all
```

After the original check fails, Funzzy prints the exact generation, job, and
commands and asks `[y/N]`. Only `y` or `yes` authorizes the commands. The
recovery runs sequentially and fail-fast, followed by one verification rerun.
A declined, skipped, headless, cancelled, or failed recovery remains a final
failure. There is no automatic acceptance and Funzzy does not infer approval
from `CI=true`.

For CI or detached watchers, opt out explicitly:

```bash
fzz run --recovery-policy skip "@quick"
fzz watch --recovery-policy skip
```

`hooks.failure` is different: it observes the final failed generation and
cannot change its result; `jobs[].recovery` runs before the final result and can
make verification pass. Service jobs cannot declare recovery.

For delayed failure notification, use the object form:

```yaml
hooks:
  failure:
    run: ./scripts/notify-failure
    settle: 30s
```

Settlement is based only on watcher generations. It does not know whether an
editor, agent, or other client is active. The command is run by the watcher as
`$SHELL -c '<command>'` from the workspace root, with inherited environment and
stdin, and its stdout/stderr are forwarded like other hooks. Funzzy injects
reserved `FUNZZY_GENERATION_ID` and `FUNZZY_GENERATION_OUTCOME` environment
variables, overriding inherited values. It does not inject failed job names,
changed paths, or evidence into argv, environment, or stdin. Use the exact
`FUNZZY_GENERATION_ID` to retrieve evidence with `fzz control output
--generation N`; latest-status lookup can race with a newer generation. A
hook that has started may have external side effects even if a newer generation
subsequently cancels it.

## 5. Recovery actions

| Symptom | Action |
|---|---|
| `fzz init` refused: `.watch.yaml` exists | deliberate create-only refusal; edit the file or `fzz config example P` to compare |
| Legacy task list needs `jobs:` | back up/commit, run `fzz migrate [-c PATH]`, inspect, then `fzz check`; section ownership edits are manual |
| Config rejected | `fzz check` names the exact path; fix, re-check |
| Target not found / ambiguous | `fzz list` shows valid targets |
| Race-like parallel failure | rerun `fzz run TARGET --sequential`; parallel fail + sequential pass is `parallel-sensitive` evidence, not a proven race |
| Watcher noise from generated files | add `ignore` or set `respect_gitignore: true` |
| Want machine-readable output | `--format toon\|json\|human` on control; `--events FILE` for run events |
