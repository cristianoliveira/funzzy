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

### 1.4 Inspect before executing

```bash
fzz list            # configured jobs (targets)
fzz explain src/lib.rs   # which jobs match/ignore a path + the filtered plan
```

### 1.5 Watch

```bash
fzz                 # zero-argument: configured watch (hero path)
fzz watch "@quick"  # watch only matching targets
```

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

**Exit codes**: 0 success/no-op, 1 workflow/operational failure, 2 usage.

## 3. Configuration guide

The preferred grouped shape (JOBS-CONFIG-CONTRACT):

```yaml
on:
  change: "src/**"        # common change globs for all jobs
  ignore: "**/*.log"      # common ignore globs
  socket: .tmp/funzzy/control.sock   # enable the control surface
  concurrency: 2          # scheduler bound (default: available parallelism)
  debounce: 500ms         # filesystem batch window (default 1s)
  watch_backend: auto     # native | poll | auto (native first, poll fallback)
  respect_gitignore: true # respect workspace .gitignore (default false)

jobs:
  - name: lint
    parallel: checks      # contiguous members may overlap
    run: cargo clippy
    cwd: packages/core    # per-job working directory
    env: { MODE: prod }   # per-job environment
    change: "src/**"      # per-job triggers
    ignore: "target/**"   # per-job ignores (strongest precedence)
    run_on_init: true     # run when the watcher starts
```

- **Matching**: a job runs when a change glob matches and no ignore glob wins.
  Explicit config `ignore` beats gitignore; gitignore applies only with
  `respect_gitignore: true` (GITIGNORE-CONTRACT).
- **Templates**: `{{filepath}}` (trigger path, backward compatible),
  `{{paths}}` (whole batch, shell-escaped), `{{relative_filepath}}`.
- **Parallel groups**: only *consecutive* jobs sharing one `parallel` name may
  overlap; reused names across a serial job start a new barrier. Order inside
  a group is unspecified (PARALLEL-EXECUTION-CONTRACT).
- **Hooks**: `on.success`/`on.failure` run after terminal generations;
  `on.close` runs one finite cleanup command only when a ready watcher shuts
  down gracefully, after active jobs/services are reaped. Finite commands do
  not run it (RUN-HOOKS-CONTRACT).
- **Legacy input**: root task lists and grouped `tasks:` remain accepted and
  are rewritten deterministically with `fzz migrate`.

## 4. Recovery actions

| Symptom | Action |
|---|---|
| `fzz init` refused: `.watch.yaml` exists | deliberate create-only refusal; edit the file or `fzz config example P` to compare |
| Legacy config needs the new shape | `fzz migrate [-c PATH]` (atomic, idempotent; `jobs:` is a no-op) |
| Config rejected | `fzz check` names the exact path; fix, re-check |
| Target not found / ambiguous | `fzz list` shows valid targets |
| Race-like parallel failure | rerun `fzz run TARGET --sequential`; parallel fail + sequential pass is `parallel-sensitive` evidence, not a proven race |
| Watcher noise from generated files | add `ignore` or set `respect_gitignore: true` |
| Want machine-readable output | `--format toon\|json\|human` on control; `--events FILE` for run events |
