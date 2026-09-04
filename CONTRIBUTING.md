# Contributing to Funzzy

Contributions, bug reports, and feature proposals are welcome.

## Setup

Funzzy requires Rust and Cargo. The minimum supported version is declared by `rust-version` in `Cargo.toml`.

```bash
git clone https://github.com/cristianoliveira/funzzy.git
cd funzzy
cargo build
```

Nix users can enter the repository development shell instead.

## Checks

Use focused tests while changing code, then run the relevant gates:

```bash
cargo test <focused-test>
make lint
make tests
make integration       # CLI, watcher, process, config, or socket behavior
```

`make integration-e2e` is reserved for changes that require real filesystem end-to-end coverage.

When `.watch.yaml` is active, the configured `@agent-final` target is the final project gate.

## Code style

- Format Rust with `cargo fmt`.
- Keep Clippy warning-free.
- Add deterministic tests for happy and failure paths.
- Use bounded polling instead of fixed sleeps in filesystem/process tests.
- Preserve public CLI, configuration, and control-socket compatibility unless the change explicitly defines a breaking contract.

See `AGENTS.md` and the nearest nested `AGENTS.md` for module-specific guidance.

## Pull requests

1. Create a focused branch.
2. Add tests before changing behavior.
3. Run the relevant checks.
4. Explain the problem, behavior change, verification, and compatibility impact.
5. Open a pull request against `main`.

Keep unrelated formatting, dependency, submodule, and generated-file changes out of the pull request.
