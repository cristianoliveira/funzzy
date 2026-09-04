# Funzzy (`fzz`)

[![Crate version](https://img.shields.io/crates/v/funzzy.svg)](https://crates.io/crates/funzzy)
[![CI Checks](https://github.com/cristianoliveira/funzzy/actions/workflows/on-push.yml/badge.svg)](https://github.com/cristianoliveira/funzzy/actions/workflows/on-push.yml)
[![Integration tests](https://github.com/cristianoliveira/funzzy/actions/workflows/on-push-integration-test.yml/badge.svg)](https://github.com/cristianoliveira/funzzy/actions/workflows/on-push-integration-test.yml)
[![Nix build](https://github.com/cristianoliveira/funzzy/actions/workflows/on-push-nixbuild.yml/badge.svg)](https://github.com/cristianoliveira/funzzy/actions/workflows/on-push-nixbuild.yml)

**One reliable edit loop for developers and coding agents.**

Funzzy is a fast Rust file watcher and local workflow runner. Define checks once, run them when files change, and inspect the exact result without scraping logs or trusting stale green output.

```text
edit -> match changed paths -> run configured jobs -> inspect fresh result -> repeat
```

## Why Funzzy?

- **One workflow:** use the same YAML for watching, one-shot local runs, editors, CI, and agents.
- **Predictable execution:** declaration order, explicit parallel groups, debounced generations, cancellation, and process-tree cleanup.
- **Useful long-running services:** readiness-enabled services can stay alive after their startup generation settles.
- **Exact feedback:** a local control socket exposes current status, generation identity, bounded output, and cancellation.

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

A secondary worktree can keep finite checks while omitting services:

```bash
fzz --no-services -- "@quick"
```

Declaration order matters. Only consecutive jobs with the same `parallel` name may overlap; other jobs create serial barriers.

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

`fzz init` never overwrites an existing configuration. Use `fzz migrate` to rewrite an accepted legacy configuration into the preferred `jobs:` form.

> [!NOTE]
> V2 (`2.0.0`) is the current line. Version 1.5.0 remains on the [`v1` branch](https://github.com/cristianoliveira/funzzy/tree/v1). See the [migration guide](docs/MIGRATION.md).

## Mental model

- A filesystem batch creates one numbered **generation**.
- Matching jobs form an ordered plan. Consecutive named parallel groups may overlap.
- A newer generation can wait for or replace active work according to the busy policy.
- Finite job timeouts terminate and reap the complete process group.
- Service readiness settles startup; service health remains a separate status view.
- Valid configuration changes reload in place. Invalid changes fail visibly instead of keeping stale behavior.
- Control clients operate on exact generations, so freshness is explicit.

## Humans and agents use the same loop

Humans can watch terminal output and press **Ctrl-G** to trigger the full pipeline. Automation can use the control socket:

```bash
fzz ctl capabilities --format toon
fzz ctl status --format toon
fzz ctl run "@quick" --wait --timeout 5m --format toon
fzz ctl output --generation 12 --tail 80 --format toon
```

Funzzy remains local and provider-neutral. It runs your commands; scripts own external services, credentials, and provider-specific polling.

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

Traditional watchers optimize for a person reading a terminal. Coding agents also need exact run identity, freshness, bounded evidence, and safe cancellation. Funzzy provides both without replacing the fast local workflow developers already know.

Inspired by [antr](https://github.com/juanibiapina/antr) and [entr](https://github.com/eradman/entr).

## License

[MIT](LICENSE)
