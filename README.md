# funzzy (fzz) [![Crate version](https://img.shields.io/crates/v/funzzy.svg?)](https://crates.io/crates/funzzy) [![Building package with nix](https://github.com/cristianoliveira/funzzy/actions/workflows/on-push-nixbuild.yml/badge.svg)](https://github.com/cristianoliveira/funzzy/actions/workflows/on-push-nixbuild.yml) [![CI integration tests](https://github.com/cristianoliveira/funzzy/actions/workflows/on-push-integration-test.yml/badge.svg)](https://github.com/cristianoliveira/funzzy/actions/workflows/on-push-integration-test.yml) [![CI Checks](https://github.com/cristianoliveira/funzzy/actions/workflows/on-push.yml/badge.svg)](https://github.com/cristianoliveira/funzzy/actions/workflows/on-push.yml)

**Run checks on every edit. Give coding agents results they can trust.**

Funzzy (`fzz`) is a fast Rust watcher for the agentic coding era. It runs local workflows as code changes and exposes exact runs, fresh results, cancellation, and bounded failure output—no log scraping, no stale green.

```bash
fzz init    # create .watch.yaml
fzz check   # validate it
fzz         # watch, run, repeat
```

One YAML workflow works for developers and coding agents. Use `fzz run TARGET` for finite execution or control a running watcher through a deterministic, machine-readable API.

> [!WARNING]
> V2 (2.0.0) is the current line. v1.5.0 remains on the [`v1` branch](https://github.com/cristianoliveira/funzzy/tree/v1); see [docs/MIGRATION.md](docs/MIGRATION.md) for the flag and config mapping.


For a workflow as simple as:

```bash
find . -name '*.ts' | fzz exec -- npx eslint {{relative_filepath}}
```

Or for more complex workflows like:
```yaml
# .watch.yml
on:
  change: ["src/**", "tests/**"]
  ignore: ["target/**", "**/*.log"]
  debounce: 500ms
  socket: .tmp/funzzy/control.sock

execution:
  concurrency: 2
  recovery_policy: prompt # prompt | skip; default prompt
  recovery_timeout: 60s # approval-only bound; default 60s

hooks:
  success: "notify-send 'checks passed'"
  failure: "notify-send 'checks failed'"

jobs:
  - name: lint @quick
    parallel: checks
    run: cargo clippy

  - name: test @quick
    parallel: checks
    run: cargo test

  - name: dev-server
    service: true
    run: cargo run
    change: "src/**"
```

Declaration order is semantic. Only consecutive jobs with same `parallel` name overlap; ordinary jobs create serial barriers. Legacy root task lists and grouped `tasks:` configs remain accepted and can be rewritten with `fzz migrate`.

## Capabilities

- **Watch or run once:** use the same workflow locally, in CI, or in an editor feedback loop.
- **Precise matching:** combine change and ignore globs, optional gitignore rules, path templates, and future-file discovery.
- **Deterministic batches:** debounce and deduplicate filesystem events before creating one generation.
- **Ordered concurrency:** run consecutive named parallel groups behind explicit serial barriers; force `--sequential` for comparison.
- **Managed processes:** cancel and reap complete process groups; opt into long-running jobs with `service: true`.
- **Finite execution deadlines:** `jobs[].timeout` terminates timed-out process trees and records the typed `timedout` outcome; control `--timeout` only bounds the caller's wait.
- **Workflow automation:** run generation-level `hooks.success` and `hooks.failure` without changing the workflow result.
- **Approved recovery:** optionally run a declared `jobs[].recovery` once after an approved failure, then verify the original job once; unanswered approval defaults to failure after `execution.recovery_timeout` (60s by default); use `--recovery-policy skip` in CI.
- **Live configuration:** valid config changes hot-reload without replacing watcher identity; invalid changes fail visibly instead of leaving stale behavior running.
- **Agent-ready control:** query capabilities, status, targets, exact generations, retained output, duration estimates, cancellation, and fresh terminal results over a permission-restricted Unix socket.
- **Observable execution:** mirror logs and append schema-versioned NDJSON events for runs, tasks, groups, services, and hooks.
- **Self-describing config:** generate schema and examples from installed binary, then validate with same parser watcher uses.
- **Lifecycle hooks:** `hooks.success`/`hooks.failure` observe generation outcomes; `hooks.close` runs one finite cleanup command when a ready watcher shuts down gracefully.

Learn more:

- [Getting started and daily workflows](docs/USAGE.md)
- [Advanced control and agent workflows](docs/ADVANCED-GUIDE.md)
- [V1 to V2 migration](docs/MIGRATION.md)
- [Configuration schema and agent discovery](docs/AGENT-CONFIG-CONTRACT.md)
- [User-approved job recovery contract](docs/JOB-RECOVERY-CONTRACT.md)
- [Current-run job duration report contract](docs/CURRENT-RUN-JOB-DURATION-REPORT-CONTRACT.md)
- [Dependency inventory and update policy](docs/DEPENDENCY-POLICY.md)
- [Pi watcher extension](pi-watcher/README.md)
- [Examples](examples/README.md)

## Enhance your workflows

Funzzy pairs well with these tools:

 - [yq](https://github.com/mikefarah/yq) - A yaml querier similar to `jq` to extract commands from GitHub Actions!

 - [nrr](https://github.com/ryanccn/nrr) - For JS/TS projects, since Funzzy runs commands on change, a faster task runner makes a difference

## Motivation

Traditional watchers are optimized for human watching terminal output. Agentic coding also needs exact run identity, freshness, cancellation, structured state, and bounded evidence—without replacing the fast local workflow developers already use.

Funzzy brings GitHub Actions-like checks into the local edit loop and makes that loop observable by both humans and coding agents. Rust keeps watcher fast and lightweight.

Funzzy is inspired by [antr](https://github.com/juanibiapina/antr) and [entr](https://github.com/eradman/entr).

## Installing

### OSX:

```bash
brew install funzzy
```

[Latest release](https://github.com/cristianoliveira/funzzy/releases):
```bash
brew install cristianoliveira/tap/funzzy
```

### Linux:

```bash
curl -s https://raw.githubusercontent.com/cristianoliveira/funzzy/main/linux-install.sh | sh
```

The installer detects the architecture (x86_64/aarch64), verifies the
published sha256 checksum, and installs both `funzzy` and `fzz` into
`/usr/local/bin`. You can pin a release:

```bash
curl -s https://raw.githubusercontent.com/cristianoliveira/funzzy/main/linux-install.sh | bash - 2.0.0
```

### Nix

```bash
nix-env -iA nixpkgs.funzzy
```

[Latest release](https://github.com/cristianoliveira/funzzy/releases):
```bash
nix profile install 'github:cristianoliveira/funzzy'
# or
nix profile install 'github:cristianoliveira/nixpkgs#funzzy'
```

Install nightly version:
```bash
nix profile install 'github:cristianoliveira/funzzy#nightly'
```

or, if you use `shell.nix`:

  ```nix
{ pkgs ? import <nixpkgs> {} }:
  pkgs.mkShell {
    buildInputs = [
      pkgs.funzzy
    ];
  };
```

### With Cargo

```bash
cargo install funzzy
```

\*Make sure you have `$HOME/.cargo/bin` in your PATH
`export PATH=$HOME/.cargo/bin:$PATH`

- From source

Make sure you have installed the following dependencies:

- Rust (>= 1.97, the minimum supported version — declared as `rust-version` in `Cargo.toml`)
- Cargo

Execute:
```
cargo install --git https://github.com/cristianoliveira/funzzy.git
```

Or, clone this repo and run:

```bash
make install
```

## Quick start

Create a config, validate it, run once, then watch — the five-step path
([full guide](docs/USAGE.md)):

```bash
fzz init                       # write a runnable .watch.yaml
fzz check                      # validate (same parser as the watcher)
fzz list                       # see the configured targets
fzz run build                  # run the exact target once, no watcher
fzz                            # zero-argument configured watch
```

The file `fzz init` writes is a **comprehensive commented starter**: a small
active hello/change example that runs immediately, plus every supported
setting documented as a comment next to its owning section (the same
metadata drives `fzz config schema`). Uncomment any documented example to
activate it; `fzz init && fzz` is the zero-dependency trial.

Both binary names work — `funzzy` and its short alias `fzz`; examples use
`fzz`. `fzz init` is create-only (it refuses an existing `.watch.yaml`);
pick a starter with `fzz init --template minimal|parallel|agent`. Rewrite a
legacy task-list config with `fzz migrate` (emits the preferred `jobs:`
form, atomically and idempotently). The installed binary is the config
reference: `fzz config schema` prints the JSON Schema and
`fzz config example minimal` prints a runnable example straight from
the parser.

### Options

Check all the options with `fzz --help`

Use a different config file:

```bash
fzz -c ~/watch.yaml
```

Fail fast stops execution when any task fails. Use it when later work depends
on every earlier task succeeding. [See its usage in our workflow](https://github.com/cristianoliveira/funzzy/blob/main/.watch.yaml#L6)

```bash
fzz --fail-fast # or fzz -b (bail)
```

Filtering tasks by target:

```bash
fzz list
fzz watch "@quick"
# Assuming one or more tasks contain `@quick`, only those tasks are watched.
```

Validate the configuration without starting a watcher:

```bash
fzz check
# Loads the same parser/validator the watcher uses: schema, globs, durations,
# concurrency, parallel groups, and path existence. Never runs tasks or opens
# a socket. Exit 0 when valid, non-zero with actionable errors when not.
```

Run same configured workflow once, without watcher or control socket:

```bash
fzz run "@quick"
# Exits with combined configured task outcome; useful in CI.
```

### Current-run job durations

Every finite local run prints one `JOB / RESULT / DURATION` row per configured
job in declaration order. A row's duration is that job's monotonic elapsed
runtime; it remains meaningful for both serial and parallel jobs. The final
`Duration:` is the separate generation wall time, never a sum of job rows.

A cancelled job that started reports its partial duration. A skipped or
never-started job prints `-` (`durationMs: null` in structured snapshots). A
recovered job has one final row whose duration includes recovery and
verification. Use the control snapshot or NDJSON `task_terminal` event when an
exact integer `durationMs` is needed.

Inspect or control a watcher configured with `on.socket`:

```bash
fzz ctl capabilities --format toon
fzz ctl status --format toon
fzz ctl run "@quick" --wait --timeout 5m --format toon
```

Run an arbitrary argv over paths from stdin:

```bash
find . -name '*.rs' | fzz exec -- cargo build
find . -name '*.[jt]s' | fzz exec -- npx eslint {{filepath}}
```

Funzzy does not implicitly invoke a shell for `exec`; use `fzz exec -- sh -c '...'` when shell operators are required.

Restart busy policy cancels and reaps active work when a newer change batch arrives.
It is useful for long-running workflows. [See more in long task test](https://github.com/cristianoliveira/funzzy/blob/main/tests/watching_with_non_block_flag.rs#L7)

```bash
fzz --on-busy restart # or fzz --restart
```

## Event batching and debounce

Filesystem events are collapsed into batches: one debounce window maps to one
generation, so a burst of writes to several files runs matching tasks **once**
per batch, not once per duplicate event. The window defaults to one second and
is configurable with `on.debounce`:

```yaml
on:
  debounce: 500ms   # <number> seconds, or <number>ms/s/m; default 1s
```

Rules:

- One batch preserves the complete normalized changed-path set (deduplicated,
deterministically ordered) and the stable event kind.
- Matching runs once per batch, never once per duplicate backend event.
- Templates expose the trigger path as `{{filepath}}` (backward compatible)
and the full batch as `{{paths}}` (shell-escaped, space-joined).
- `control emit` is an **explicit immediate event**: it routes through the
same matching and busy-run policy as a native batch but does not wait for the
debounce window.
- Invalid `on.debounce` values (zero, negative, unknown suffix) fail loudly;
they never silently change timing.

## Filesystem backend policy

By default Funzzy uses the native filesystem backend and **automatically falls
back to deterministic polling** if native watch registration fails (containers,
network filesystems, WSL, unusual platforms). You can force a backend:

```yaml
on:
  watch_backend: auto     # native first, poll fallback (default)
  # watch_backend: native  # fail clearly if native is unavailable
  # watch_backend: poll    # always poll
  poll_interval: 200ms    # used with poll; default 500ms
```

Polling scans watched roots for create/modify/remove changes on a fixed
interval and feeds the exact same batching, matching, and execution path as
the native backend. Tradeoffs: polling adds a small per-interval scan cost and
change latency up to the interval; prefer native for large trees. Forced
native fails with an actionable error instead of silently changing semantics.

## Agents and configuration discovery

Agents (and humans) discover the current configuration surface from the
**installed binary**, never from stale docs. Bootstrap commands:

```sh
fzz config schema                    # full JSON Schema for the preferred jobs: format
fzz config schema --section parallel # one bounded section + hint for the full schema
fzz config example minimal           # runnable minimal .watch.yaml
fzz config example agent             # control-socket + verify-style example
fzz check                            # validate .watch.yaml (semantic checks)
fzz list | fzz explain PATH          # inspect what would run
fzz run TARGET | fzz watch           # execute
```

All `config` commands are non-interactive and side-effect-free: they never
read a project config, start a watcher, open a socket, or run tasks. The
schema is the single source of truth for structure; `fzz check` adds semantic
validation. Legacy root-list configs remain accepted and are rewritten with `fzz migrate`.

```yaml
hooks:
  success: ./scripts/notify-success   # once per passing generation
  failure: ./scripts/notify-failure   # once per failing generation
  close: ./scripts/cleanup            # once per graceful ready-watcher close
```

See [run and watcher hook lifecycle](docs/RUN-HOOKS-CONTRACT.md) for signal,
timeout, reload, and exit-code semantics.

## Parallel execution

Funzzy can run independent tasks concurrently. Concurrency is **opt-in**: a
task belongs to a named `parallel` group, and only *consecutive* tasks sharing
the same group name may overlap. This keeps existing sequential configs
running exactly as before — no migration needed.

```yaml
on:
  change: "src/**"
execution:
  concurrency: 4      # optional global cap on active tasks
jobs:
  - name: lint
    parallel: checks  # group membership is explicit
    run: cargo clippy
  - name: test
    parallel: checks
    run: cargo test
  - name: package
    run: cargo build   # serial: runs alone, after the group
```

## Finite job execution deadlines

Use `jobs[].timeout` to bound a finite job's complete invocation:

```yaml
jobs:
  - name: await-remote
    trigger: manual
    timeout: 30m
    run: ./scripts/await-remote.sh
```

The execution timeout starts at the first successful spawn. When it elapses,
Funzzy terminates the complete process group (graceful `SIGTERM`, escalating
to `SIGKILL` when needed), reaps it, retains bounded output, and records the
job's typed `timedout` state in the failed generation. Use the exact generation
with `fzz control output --generation N` to retrieve pre-termination evidence.

This is separate from control `--timeout`: that option bounds only how long a
caller waits for `control await` or `control run --wait`; it never terminates
the child. A timeout in the job configuration owns child lifetime, while a
client timeout owns caller patience. See the full provider-neutral recipe in
[the advanced guide](docs/ADVANCED-GUIDE.md).

## Troubleshooting

#### Why the watcher is running the same task multiple times?

This might be due to different causes, the most common issue when using VIM is because of its default backup setting
which causes changes to multiple files on save. (See [Why does Vim save files with a ~ extension?](https://stackoverflow.com/questions/607435/why-does-vim-save-files-with-a-extension/607474#607474)).
For such cases either disable the backup or [ignore them in your watch rules](https://github.com/cristianoliveira/funzzy/blob/main/examples/jobs-with-long-running-commands.yaml#L5).

For other cases use the verbose `fzz -V | grep 'Triggered by'` to understand what is triggering a task to be executed.

## Automated tests

Running unit tests:

```bash
cargo test
```

or simple `make tests`

Running integration tests:

```
make integration
```

## Code Style

We use `rustfmt` to format the code. To format the code run:

```bash
cargo fmt
```

## Contributing

- Fork it!
- Create your feature branch: `git checkout -b my-new-feature`
- Commit your changes: `git commit -am 'Add some feature'`
- Push to the branch: `git push origin my-new-feature`
- Submit a pull request

### Want to help?

 - Open pull requests
 - Create Issues
 - Report bugs
 - Suggest new features or enhancements

Any help is appreciated!

**Pull Request should have unit tests**

# License

This project was made under MIT License.
