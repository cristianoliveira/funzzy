# Contributing to Funzzy

Build Funzzy, test the behavior you change, then open a focused pull request against `main`. Bug reports and feature proposals are also welcome.

## Setup

Install Rust and Cargo first. `Cargo.toml` lists the minimum Rust version in `rust-version`.

```bash
git clone https://github.com/cristianoliveira/funzzy.git
cd funzzy
cargo build
```

If you use Nix, enter the repository development shell instead.

## Checks

Run focused tests while you change code. Then run the applicable gates:

```bash
cargo test <focused-test>
make lint
make tests
make integration       # CLI, watcher, process, config, or socket behavior
```

Use `make integration-e2e` only when a change requires a real file-system test.

If `.watch.yaml` is active, run the configured `@agent-final` target as the final project gate.

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
