# funzzy (fzz) [![Crate version](https://img.shields.io/crates/v/funzzy.svg?)](https://crates.io/crates/funzzy) [![Building package with nix](https://github.com/cristianoliveira/funzzy/actions/workflows/on-push-nixbuild.yml/badge.svg)](https://github.com/cristianoliveira/funzzy/actions/workflows/on-push-nixbuild.yml) [![CI integration tests](https://github.com/cristianoliveira/funzzy/actions/workflows/on-push-integration-test.yml/badge.svg)](https://github.com/cristianoliveira/funzzy/actions/workflows/on-push-integration-test.yml) [![CI Checks](https://github.com/cristianoliveira/funzzy/actions/workflows/on-push.yml/badge.svg)](https://github.com/cristianoliveira/funzzy/actions/workflows/on-push.yml)

A lightweight blazingly fast file watcher inspired by [antr](https://github.com/juanibiapina/antr) and [entr](https://github.com/eradman/entr).

Configure auto-execution of different commands using semantic YAML and [Unix shell style pattern match](https://en.wikipedia.org/wiki/Glob_(programming)) or stdin.

> [!WARNING]
> ### Version 2.0.0
> These are instructions of the new V2 version yet to be released, check [V1](https://github.com/cristianoliveira/funzzy/tree/v1) if you are using v1.5.0

For a workflow as simple as:
```bash
find . -name '*.ts' | funzzy 'npx eslint {{relative_path}}'
```

Or more complex workflows like:
```yaml
# .watch.yaml (or .watch.yml)
# list here all the events and the commands that it should execute
# TIP: include '.watch.yaml' in your .git/info/exclude to ignore it.
# TIP2: List the tasks/steps from quicker to slower for better workflows
#
# Run: `fzz --fail-fast --restart` to restart busy runs on change.

- name: run my tests
  run: make test
  change: "tests/**"
  ignore: "tests/integration/**"
  run_on_init: true

- name: Starwars ascii art
  run: telnet towel.blinkenlights.nl
  change:
    - "/tmp/starwars.txt"
    - ".watch.yaml"

# Command path templates for custom scripts
- name: run test & linter for a single file
  run:
   - "npm run lint -- {{relative_path}}",
   - "npm test -- $(echo '{{absolute_path}}' | sed -r s/.(j|t)sx?//)"
  change: ["src/**", "libs/**"]
  ignore: ["src/**/*.stories.*", "libs/**/*.log"]

- name: run ci checks @quick @ci
  run: | ## Watch with `fzz watch @ci`
   cat .github/workflows/on-push.yml \
    | yq '.jobs | .[] | .steps | .[] | .run | select(. != null)' \
    | xargs -I {} bash -c {}
  change: "src/**"
  run_on_init: true

- name: finally stage the changed files in git
  run:
    - git add {{relative_path}}
    - git commit
  change:
    - "src/**"
    - "tests/**"
  ignore: "**/*.log"
```

**New in v1.6.0**: Common rules format to reduce duplication!
```yaml
# Share common watch patterns across tasks
on:
  change: ["src/**", "lib/**"]
  ignore: ["**/*.log"]

tasks:
  - name: build
    run: cargo build
  - name: test
    run: cargo test
    change: "tests/**"  # Override for specific task
```

** Nested groups for organizing tasks by domain!
```yaml
# Frontend tasks
- on:
    change: ["src/frontend/**"]
  tasks:
    - name: frontend-build
      run: npm run build

# Backend tasks
- on:
    change: ["src/backend/**"]
  tasks:
    - name: backend-build
      run: cargo build
```

See more:

 - [Documentation](/docs/USAGE.md)
 - [Check our workflow in funzzy](https://github.com/cristianoliveira/funzzy/blob/master/.watch.yaml#L6) :)
 - [Check the examples folder](https://github.com/cristianoliveira/funzzy/tree/master/examples)

### Enhance your workflows

Funzzy pairs well with these tools:

 - [yq](https://github.com/mikefarah/yq) - A yaml querier similar to `jq` to extract commands from GitHub Actions!

 - [nrr](https://github.com/ryanccn/nrr) - For JS/TS projects, since Funzzy runs commands on change, a faster task runner makes a difference

## Motivation

To create a lightweight watcher that **allows me to set up personal local workflows with specific automated checks and steps, similar to GitHub Actions**.
Funzzy was built with Rust, which makes it blazingly fast and light.

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
curl -s https://raw.githubusercontent.com/cristianoliveira/funzzy/master/linux-install.sh | sh
```

You can specify the versions:
```bash
curl -s https://raw.githubusercontent.com/cristianoliveira/funzzy/master/linux-install.sh | bash - 1.0.0
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

Both binary names work — `funzzy` and its short alias `fzz`; examples use
`fzz`. Migrate a legacy task-list config with `fzz init --migrate` (emits the
preferred `jobs:` form). The installed binary is the config reference:
`fzz config schema` prints the JSON Schema and `fzz config example minimal`
prints a runnable example — docs never drift from the parser.

### Options

Check all the options with `fzz --help`

Use a different config file:

```bash
fzz -c ~/watch.yaml
```

Fail fast which bails the execution if any task fails. Useful for workflows that
depend on all task to be successful. [See its usage in our workflow](https://github.com/cristianoliveira/funzzy/blob/master/.watch.yaml#L6)

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

Run with some arbitrary command and stdin

```bash
find . -name '*.rs' | fzz 'cargo build'
```

Templates for composing commands

```bash
find . -name '*.[jt]s' | fzz 'npx eslint {{filepath}}'
```

Restart busy policy: cancels the currently running task when new change events arrive from files.
It is super useful when a workflow contains long-running tasks. [See more in long task test](https://github.com/cristianoliveira/funzzy/blob/master/tests/watching_with_non_block_flag.rs#L7)

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
validation. Legacy root-list configs remain accepted and are migrated with
`fzz init --migrate`.

## Parallel execution

Funzzy can run independent tasks concurrently. Concurrency is **opt-in**: a
task belongs to a named `parallel` group, and only *consecutive* tasks sharing
the same group name may overlap. This keeps existing sequential configs
running exactly as before — no migration needed.

```yaml
on:
  change: "src/**"
  concurrency: 4      # optional global cap on active tasks
tasks:
  - name: lint
    parallel: checks  # group membership is explicit
    run: cargo clippy
  - name: test
    parallel: checks
    run: cargo test
  - name: package
    run: cargo build   # serial: runs alone, after the group
```

Key rules:

- **Contiguous groups**: only consecutive tasks with the same group name
  share one barrier. `A@x, B, C@x` runs `A -> B -> C` as two *separate*
  `x` occurrences; the reused name never reconnects across a serial task.
- **Barriers**: commands inside one task stay strictly sequential; a serial
task (no `parallel`) runs alone between groups.
- **Filtering**: target selection keeps the original topology. If only one
  group member matches, it runs alone — barriers stay valid.
- **`on.concurrency`**: global cap on simultaneously active tasks. Defaults
to available parallelism, resolved once at plan time. `1` is valid and means
tasks run one at a time inside the barrier. Without `parallel` groups,
concurrency is never inferred.
- **Failures**: by default a failed task does not stop siblings or later
stages; the run fails with combined results. `--fail-fast` cancels active
siblings and skips queued/later work on the first failure.
- **Restart** (`--restart`): a new event cancels and reaps all active tasks
across every group, then starts the newest generation.
- **Output**: live lines from group tasks carry a `[task]` prefix so
interleaved output keeps identity; the final summary lists every task with
its group. Ordering inside a group is intentionally unspecified.
- **Diagnosing races**: rerun the same target with `--sequential`
(`fzz run TARGET --sequential` or `fzz ctl run TARGET --sequential --wait`)
to compare against effective concurrency one. Parallel fail + sequential pass
is `parallel-sensitive` evidence, not proof of a race root cause.

**Cost guidance**: parallelism helps when independent tasks dominate a batch
— latency approaches the slowest task rather than the sum. It does **not**
make every workload faster: CPU-bound tasks on few cores, or tasks competing
for one resource (a database, a port, a lock file), can slow down. Start with
`concurrency: 2` and measure; raise only when independent work is proven.
Each concurrent task is a separate child process with its own process group.

## Troubleshooting

#### Why the watcher is running the same task multiple times?

This might be due to different causes, the most common issue when using VIM is because of its default backup setting
which causes changes to multiple files on save. (See [Why does Vim save files with a ~ extension?](https://stackoverflow.com/questions/607435/why-does-vim-save-files-with-a-extension/607474#607474)).
For such cases either disable the backup or [ignore them in your watch rules](https://github.com/cristianoliveira/funzzy/blob/master/examples/tasks-with-long-running-commands.yaml#L5).

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
