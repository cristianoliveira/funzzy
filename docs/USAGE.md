# Use Funzzy day to day

Create a configuration, check it, run a job once, then start the watcher. These five steps give you the shortest path to a working edit loop.

This guide uses `fzz`. The `funzzy` binary works the same way. If you used V1, see the [migration guide](MIGRATION.md) before you change your commands.

## 1. Getting started (five steps)

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

### 1.4 Inspect before executing

```bash
fzz list            # configured jobs (targets)
fzz explain src/lib.rs   # which jobs match/ignore a path + the filtered plan
```

### 1.5 Watch

```bash
fzz                 # zero-argument: configured watch (hero path)
fzz watch "@quick"  # watch only matching targets
fzz -- "@quick"     # equivalent watch shorthand; target follows `--`
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

The root shorthand accepts zero or one target only after the end-of-options
delimiter. `fzz --` is equivalent to `fzz`; root watch options compose before
the delimiter, for example `fzz --no-services -- @quick`. Everything after
the delimiter is target data, so `fzz -- --service` selects a target named
`--service`; extra values are usage errors. Without the delimiter, a word
matching a subcommand still invokes that subcommand.

The edit loop is now ready: configure, check, run once, then watch.

### Trigger the pipeline from the keyboard

While Funzzy watches files, press **Ctrl-G** once to run all configured jobs. If a generation is active, Funzzy saves one request. It starts that request after the active generation finishes. Funzzy ignores more requests during this time. Ctrl-C starts a graceful shutdown. Piped input accepts the same Ctrl-G byte. Closed input does not cause an error.

### Readiness-enabled services

Add a `readiness` command when you want the watcher to settle a generation after it confirms service health:

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

After the probe passes, `api` moves to the pool of services that the worker owns. The generation becomes `passed`. Control status reports the separate view `services: [{name: api, state: ready}]`. A later restart or failure does not change that generation result.

If the probe fails or times out, the generation fails and Funzzy reaps the service. Readiness configuration is strict and stays fixed for that generation. Without `readiness`, a legacy service keeps its generation running while the service is alive.

### Current-run job durations

A finite run shows one `JOB / RESULT / DURATION` row per configured job, in declaration order. Each duration measures that job's elapsed time, including serial or parallel work. The generation's `Duration:` is separate wall-clock time; do not add the job rows to calculate it.

A cancelled job that started keeps a partial duration. A skipped or unstarted job shows `-`; snapshots and events report `durationMs: null`. A recovered job appears once, with its final state and duration through final verification. A service with `readiness` records duration until its probe passes; later uptime belongs to pool state. A legacy live service has no finite duration while it runs. For integer milliseconds, inspect `tasks[].durationMs` in a control snapshot or a `task_terminal` NDJSON event.

## 2. Daily workflow decision table

| Goal | Command | Watcher? | Side effects |
| --- | --- | --- | --- |
| Watch + run on change | `fzz` / `fzz watch [TARGET]` / `fzz -- [TARGET]` | yes | runs tasks, may open socket |
| Run once, finite | `fzz run TARGET` | no | runs tasks, exits |
| Validate config | `fzz check [-c PATH]` | no | none |
| List targets | `fzz list` | no | none |
| Explain a path | `fzz explain PATH` | no | none |
| Ad-hoc over stdin | `fzz exec -- PROGRAM ARG...` | no | runs PROGRAM per stdin path |
| Control running watcher | `fzz control status\|list\|run\|retry\|emit\|await\|cancel\|output\|capabilities` | no | talks to the socket |
| Init a starter config | `fzz init [--template P]` | no | writes `.watch.yaml` (create-only, refuses existing) |
| Migrate a legacy config | `fzz migrate [-c PATH]` | no | atomic in-place rewrite (idempotent) |
| Config discovery | `fzz config schema\|example` | no | none (never reads project config) |
| Print agent instructions | `fzz --agents-instruction` | no | prints Markdown; no file changes |

### Retry failed jobs from the latest run

With no generation, Funzzy atomically selects the latest terminal generation.
It never searches for an older failure: if the latest run passed, there is
nothing to retry. Only that generation's failed or timed-out finite jobs are
retried; passed, skipped, and cancelled jobs stay out. Services cannot be retried.

```sh
fzz control --format toon retry
# Copy the sourceGeneration N and new runId M from the response.
fzz control --format toon await --generation M --timeout 5m
fzz control --format toon output --generation M --tail 80

# To require a particular observed generation instead:
fzz control --format toon retry --generation N
```

`--format` belongs before the `control` action. `--wait --timeout 5m`
combines scheduling and observation; a timeout leaves the retry running, so
await its returned `runId` again. Bare retry always evaluates the latest
source under the watcher's schedule lock. `--generation N` requires that exact
source; an already-accepted retry of N returns the same runId while cached,
whereas an uncached stale source fails. `watcher_busy` means wait for active
work or pending batch routing. For `stale_revision` or `source_unavailable`,
observe fresh work. `no_retryable_jobs` means no finite failure is eligible;
`unsupported_service` means a failed service was selected.

**Busy policy**: `--on-busy wait|restart` (default `wait`); `--restart`
cancels and reaps active work on a newer event. **Fail fast**: `--fail-fast`
stops at the first failing task. **Logging**: `--log-file FILE` mirrors all
output; **events**: `--events FILE` appends NDJSON run events.

**Exit codes**: 0 success/no-op, 1 workflow/operational failure, 2 usage. `--recovery-policy prompt|skip` overrides the configured policy for `fzz run` and watch sessions; it does not edit the config.

### Agent instructions

For an already configured project, run `fzz --agents-instruction` and paste its output into that project's `AGENTS.md`. It teaches agents to use `fzz ctl` to observe the running watcher before editing, wait for automatic feedback after editing, inspect failures, and trigger an explicit target when no run matched. The command only prints text; it needs neither `.watch.yaml` nor an active watcher. Pi watcher tools are an optional enhancement, not a requirement. See the [advanced guide](ADVANCED-GUIDE.md#3-agent-edit-feedback-loop) for control-socket details.

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
  env:                     # defaults for jobs and all configured hooks
    APP_MODE: test
    OPTIONAL_FLAG: ''       # empty values are valid

hooks:
  success: echo "checks passed"
  failure: echo "checks failed"
  close: echo "watcher stopped"

jobs:
  - name: lint
    parallel: checks      # contiguous members may overlap
    run: cargo clippy
    cwd: packages/core    # per-job working directory
    env: { MODE: prod }   # per-job overrides execution.env
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
- **Parallel groups**: only *consecutive* jobs sharing one `parallel` name may
  overlap; reused names across a serial job start a new barrier. Order inside
  a group is unspecified (PARALLEL-EXECUTION-CONTRACT).
- **Environment**: `execution.env` sets string defaults for configured jobs, services, readiness probes, recovery commands, and success/failure/close hooks. Children also inherit parent variables. A job's `env` overrides defaults for that job and its readiness or recovery commands.

  Funzzy reserves `FUNZZY_GENERATION_ID` and `FUNZZY_GENERATION_OUTCOME` for hooks. Its values override inherited or configured values. For filesystem-triggered job children, Funzzy also reserves `FZZ_TRIGGER_PATH`, `FZZ_TRIGGER_RELATIVE_PATH`, and `FZZ_TRIGGER_PATHS_FILE`. It sets those values after configured values. Pathless runs remove stale values, and hooks do not receive trigger values implicitly.

  Funzzy does not change the process-wide environment. Empty values are valid. Names must match `[A-Za-z_][A-Za-z0-9_]*`; values cannot contain NUL. Ad-hoc `fzz exec` does not load project configuration.

### Changed-path inputs

For new job scripts, prefer child environment data over command interpolation:

| Input | Meaning |
| --- | --- |
| `FZZ_TRIGGER_PATH` | One trigger path. |
| `FZZ_TRIGGER_RELATIVE_PATH` | The trigger path relative to that job's `cwd`. |
| `FZZ_TRIGGER_PATHS_FILE` | Absolute pointer to a private JSON array containing the complete changed-path batch. |

The path templates `{{filepath}}`, `{{absolute_path}}`,
`{{relative_filepath}}`, `{{relative_path}}`, and `{{paths}}` are deprecated
for **new job scripts**. Supported spacing variants, such as `{{ filepath }}`
and `{{ paths }}`, have the same status. They are not removed: existing
commands execute as before, without a new runtime warning or automatic rewrite.

For example, if one batch contains `/repo/docs/two words.md` and
`/repo/src/a.rs`, `{{paths}}` expands to this shell-escaped, space-joined text:

```text
'/repo/docs/two words.md' '/repo/src/a.rs'
```

A shell command interprets that text as two path arguments. A direct-argv
command keeps the complete expansion as **one** argv element; Funzzy does not
split it into two arguments. The preferred batch file instead contains literal
JSON strings, not shell-escaped command text:

```json
["/repo/docs/two words.md", "/repo/src/a.rs"]
```
- **Trigger path variable**: a filesystem-triggered shell or direct-argv child receives `FZZ_TRIGGER_PATH`. Its value matches `{{filepath}}` for that run: **one** path, not the complete batch. Use `FZZ_TRIGGER_PATHS_FILE` for the batch. The path is data, not command text, so a script can read paths with spaces or shell characters safely.

  Init, manual target, and Ctrl-G runs have no trigger path. Funzzy removes a stale `FZZ_TRIGGER_PATH` even if the parent or configured `env` set one. Hooks do not receive it implicitly. To migrate, remove `{{filepath}}` from `run:` and read the variable in the script:

  ```yaml
  # before (deprecated interpolation)
  - name: checks
    run: './scripts/check.sh {{filepath}}'
  # after (path arrives as environment data)
  - name: checks
    run: './scripts/check.sh'
  ```

  The script reads it safely and handles a pathless run:

  ```sh
  # scripts/check.sh
  if [ -z "${FZZ_TRIGGER_PATH:-}" ]; then
    echo "no trigger path (init or manual run)" >&2
    exit 0
  fi
  "$SHELL" -n "$FZZ_TRIGGER_PATH"
  ```

  The cwd-relative form works the same way: every triggered child also
  receives `FZZ_TRIGGER_RELATIVE_PATH` with the exact value
  `{{relative_filepath}}` expands to for that job's working directory. Jobs
  with different `cwd` can receive different values. As with the legacy
  template, a trigger outside a job's `cwd` keeps its absolute path instead
  of using `../` segments. Funzzy does not inject either variable into hooks.
  Migrate `{{relative_filepath}}` the same way — no interpolation in `run:`:

  ```yaml
  # before (deprecated interpolation)
  - name: lint-changed
    run: './scripts/lint-one.sh {{relative_filepath}}'
  # after (path arrives as environment data)
  - name: lint-changed
    run: './scripts/lint-one.sh'
  ```

  ```sh
  # scripts/lint-one.sh
  if [ -z "${FZZ_TRIGGER_RELATIVE_PATH:-}" ]; then
    echo "no trigger path (init or manual run)" >&2
    exit 0
  fi
  "$SHELL" -n "$FZZ_TRIGGER_RELATIVE_PATH"
  ```
- **Changed-path batch file**: each filesystem-triggered shell or direct-argv job receives `FZZ_TRIGGER_PATHS_FILE`. This absolute path points to a private, read-only UTF-8 JSON file. The array contains the complete changed set after debounce, sorted and deduplicated. Each entry is a literal normalized path. Funzzy does not shell-quote entries, filter them for a job, or rebase them for `cwd`.

  Job commands, recovery commands, readiness probes, and supervised services can read the same immutable file for that generation. A retry receives the same array in a new file. Read it while Funzzy supervises the child. Do not rely on its file name or on access after the job or service exits.

  Pathless runs do not receive the variable; hooks do not receive it implicitly. The watcher does not currently include native non-UTF-8 paths in its batch model.

  Migrate a whole-batch command without interpolating `{{paths}}`:

  ```yaml
  # before (supported, deprecated interpolation)
  - name: check-changed
    run: './scripts/check-changed.sh {{paths}}'
  # after (structured data, identical for shell and direct argv jobs)
  - name: check-changed
    run: 'node ./scripts/check-changed.mjs'
  ```

  ```js
  // scripts/check-changed.mjs
  import { readFile } from "node:fs/promises";

  const file = process.env.FZZ_TRIGGER_PATHS_FILE;
  if (!file) process.exit(0); // init or manual run

  const paths = JSON.parse(await readFile(file, "utf8"));
  for (const path of paths) console.log(path);
  ```

  `{{paths}}` remains supported with its existing shell-escaped, space-joined
  expansion. In direct-argv commands it remains one expanded argv element; it
  does not become multiple arguments. It is deprecated only as an input for
  new job scripts. Prefer the JSON file when a reusable script needs structured
  batch data or filenames with unusual characters.
- **Hooks**: `hooks.success` runs after a passing generation. Scalar `hooks.failure` runs immediately after failure. Object-form `hooks.failure.run` waits for `settle` only while that failed watcher generation remains latest. These hooks do not change the result. `hooks.close` runs once when a ready watcher shuts down gracefully, after it reaps active jobs and services. Finite commands do not run settled or close hooks (RUN-HOOKS-CONTRACT).
- **Recovery**: `jobs[].recovery` accepts one command or an ordered list. A failed finite job can recover only with `execution.recovery_policy: prompt` and an attached TTY answer of `y` or `yes`. Funzzy runs recovery once, then verifies the original job once. `n`, EOF, invalid input, no TTY, or `skip` keeps the failure and does not spawn recovery.
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
`$SHELL -c '<command>'` from the workspace root, with inherited environment,
`execution.env` defaults, and inherited stdin. Its stdout/stderr are forwarded
like other hooks. Funzzy injects reserved `FUNZZY_GENERATION_ID` and
`FUNZZY_GENERATION_OUTCOME` environment variables, overriding configured and
inherited values. It does not inject failed job names,
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
