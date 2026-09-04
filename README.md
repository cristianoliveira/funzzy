# Funzzy (`fzz`)

[![Crate version](https://img.shields.io/crates/v/funzzy.svg)](https://crates.io/crates/funzzy)
[![CI Checks](https://github.com/cristianoliveira/funzzy/actions/workflows/on-push.yml/badge.svg)](https://github.com/cristianoliveira/funzzy/actions/workflows/on-push.yml)
[![Integration tests](https://github.com/cristianoliveira/funzzy/actions/workflows/on-push-integration-test.yml/badge.svg)](https://github.com/cristianoliveira/funzzy/actions/workflows/on-push-integration-test.yml)
[![Nix build](https://github.com/cristianoliveira/funzzy/actions/workflows/on-push-nixbuild.yml/badge.svg)](https://github.com/cristianoliveira/funzzy/actions/workflows/on-push-nixbuild.yml)

**One reliable edit loop for developers and coding agents.**

Funzzy is a fast Rust file watcher and local workflow runner. You define checks in one YAML file. Funzzy runs matching jobs when files change. It reports the state of each run, so you do not have to inspect logs or reuse an old result.

```text
edit -> match changed files -> run jobs -> inspect result -> repeat
```

## Why Funzzy?

- **One workflow:** use the same YAML file for watch mode, local runs, editors, CI, and agents.
- **Predictable execution:** Funzzy runs jobs in declared order. Named groups enable parallel work. Cancellation stops the complete process tree.
- **Long-running services:** a readiness check confirms that a service started correctly. The service can then stay active.
- **Exact feedback:** local tools can use a control socket. They can read status and output or cancel a specific run.

Funzzy has two equivalent binary names: `funzzy` and the shorter `fzz`. This README uses `fzz`.

## A quick glimpse

Create `.watch.yaml`:

```yaml
on:
  change: ["src/**", "tests/**"]
  ignore: ["target/**"]
  socket: .tmp/funzzy/control.sock

execution:
  concurrency: 2

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
    readiness:
      run: curl --fail http://127.0.0.1:3000/health
      timeout: 30s
      interval: 500ms
```

Then use one of four paths:

```bash
fzz check                 # validate without running anything
fzz                       # watch the complete workflow
fzz -- "@quick"           # concise alias for: fzz watch "@quick"
fzz run "@quick"          # run once and exit
```

A secondary worktree can run checks without starting services:

```bash
fzz --no-services -- "@quick"
```

Declaration order is important. Consecutive jobs with the same `parallel` name can run at the same time. Other jobs run in sequence.

## Start in one minute

Install Funzzy with one of these commands:

```bash
brew install cristianoliveira/tap/funzzy
cargo install funzzy
curl -s https://raw.githubusercontent.com/cristianoliveira/funzzy/main/linux-install.sh | sh
```

See [Installation](docs/INSTALLATION.md) for Nix, pinned releases, source builds, and platform details.

Create and inspect a workflow:

```bash
fzz init                         # create a commented .watch.yaml
fzz config example minimal       # print a small runnable example
fzz config schema                # print the current JSON Schema
fzz check                        # validate with the production parser
fzz list                         # list configured targets
fzz run build                    # finite local run
fzz                              # start watching
```

`fzz init` does not overwrite an existing configuration. Use `fzz migrate` to convert an accepted old configuration to the `jobs:` form.

> [!NOTE]
> V2 (`2.0.0`) is the current line. Version 1.5.0 remains on the [`v1` branch](https://github.com/cristianoliveira/funzzy/tree/v1). See the [migration guide](docs/MIGRATION.md).

## How it works

- Funzzy groups file events into a batch. Each batch gets a **generation** number.
- Matching jobs form an ordered plan. Consecutive jobs in the same named group can run in parallel.
- The busy policy controls what happens when a new generation starts. Funzzy waits for active work or replaces it.
- A finite job must exit. Its timeout stops and reaps the complete process group.
- A readiness check confirms service startup. Service health then has a separate status.
- Funzzy reloads valid configuration changes. It stops with a clear error for an invalid change.
- Control clients use an exact generation number. This number identifies one result.

## People and agents use the same loop

You can read terminal output and press **Ctrl-G** to start all configured jobs. An agent can use the control socket:

```bash
fzz ctl capabilities --format toon
fzz ctl status --format toon
fzz ctl run "@quick" --wait --timeout 5m --format toon
fzz ctl output --generation 12 --tail 80 --format toon
```

Funzzy runs local commands and does not depend on a CI provider. Your scripts manage external services, credentials, and provider requests.

## Learn more

| Need | Guide |
| --- | --- |
| Installation and upgrades | [Installation](docs/INSTALLATION.md) |
| First workflow and daily commands | [Usage](docs/USAGE.md) |
| Control socket, agents, parallelism, and troubleshooting | [Advanced guide](docs/ADVANCED-GUIDE.md) |
| V1 to V2 changes | [Migration](docs/MIGRATION.md) |
| Service lifecycle and readiness | [Service lifecycle contract](docs/SERVICE-LIFECYCLE-CONTRACT.md) |
| Job recovery | [Recovery contract](docs/JOB-RECOVERY-CONTRACT.md) |
| Configuration discovery for agents | [Agent configuration contract](docs/AGENT-CONFIG-CONTRACT.md) |
| Example workflows | [Examples](examples/README.md) |
| Pi integration | [pi-watcher](pi-watcher/README.md) |
| Development and pull requests | [Contributing](CONTRIBUTING.md) |

## Why it exists

Traditional file watchers show output in a terminal. Coding agents also need an exact run number, current results, short failure output, and safe cancellation. Funzzy provides these functions in the same local workflow.

Inspired by [antr](https://github.com/juanibiapina/antr) and [entr](https://github.com/eradman/entr).

## License

[MIT](LICENSE)
