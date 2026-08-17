# CLI command layer

Owns command implementations selected by `src/main.rs`.

## Modules

- `init.rs`: create-only default config from the shared template profiles (`templates.rs`).
- `migrate.rs`: explicit `fzz migrate` rewrite of accepted legacy config (pure transform + atomic CLI adapter).
- `watch.rs`: blocking watch loop; execute matched commands serially.
- `watch_non_block.rs`: cancellable watch loop; wire `Worker`, `ControlState`, and optional control server.
- `mod.rs`: `Command` contract and command exports.

## Boundaries

- CLI commands orchestrate `Rules`, `Watches`, watcher, worker, and presentation; they do not redefine parsing or matching policy.
- Both watch modes must agree on readiness, `run_on_init`, command templating, path triggers, fail-fast, and result presentation.
- Start control socket only after filesystem watches are registered so socket readiness means watcher readiness.
- A configured control socket implies non-block behavior; keep that selection in composition root.
- Keep migration content/comments intact unless migration contract changes deliberately.

When behavior applies to both watch modes, extract shared policy rather than fixing only one path.

## Verification

Write focused unit tests for command-local seams and integration tests for observable CLI behavior.

```sh
cargo test cli
cargo test --features test-integration
make lint
```

For init changes, run `command_init*` tests. For watch-mode changes, cover blocking and non-block paths plus unhappy path.
