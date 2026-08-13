# Rust application modules

## Execution shape

```text
main.rs
  -> rules.rs / yaml.rs
  -> watches.rs <- watcher.rs
  -> cli/watch.rs | cli/watch_non_block.rs
  -> cmd.rs | workers.rs
  -> control.rs
```

`main.rs` is composition root: parse CLI, choose config or stdin input, select target, choose blocking/non-block execution, and wire reload/logging behavior. Keep concrete wiring here; move reusable policy to owning module.

## Main modules

### Workflow tasks — `rules.rs`, `yaml.rs`

Own config parsing, task validation, glob semantics, shared `on` rules, command extraction, and path-template expansion. A `Rules` value represents one configured task despite legacy plural name.

- Keep YAML shape handling in parser functions and task invariants on `Rules`.
- Preserve relative versus absolute glob behavior.
- Test malformed and valid forms, including legacy lists and grouped `on`/`tasks`.
- Do not move filesystem event or process behavior here.

### Watch plan — `watches.rs`, `watcher.rs`

`Watches` maps tasks to concrete watch roots and selects tasks for changed paths. `watcher.rs` is only filesystem-event adapter.

- Ignore match wins before change match.
- Normalize both project-relative and absolute paths explicitly.
- Register every watch before signaling readiness; initialization must not create event-loss gap.
- Keep `notify` details out of rule policy.

### Run lifecycle — `workers.rs`, `cmd.rs`

`Worker` schedules sequential commands, assigns generation IDs, supports cancellation, and emits `WorkerEvent`. `cmd.rs` owns child-process wrapper and shell execution.

- Preserve sequential command order and fail-fast semantics.
- A newer non-block run cancels active child before replacement work.
- Treat generation as external run identity; never reuse or decrement it.
- Keep process errors visible as failures, not silent skips.

### Control API — `control.rs`

Projects `WorkerEvent` into `ControlState` and exposes JSON-RPC 2.0 over permission-restricted Unix socket.

- Keep transport validation separate from state transition logic.
- Notifications have no response; requests preserve caller ID.
- `run` returns scheduled generation so clients can await exact work.
- Wire-format changes require matching Pi watcher contract changes and integration tests.

### Cross-cutting adapters

- `stdout.rs`, `logging.rs`: console presentation and optional mirrored log.
- `errors.rs`: `FzzError`, context, and actionable hints.
- `environment.rs`: environment feature switches.
- `lib.rs`: module exports; it is not composition root for binary behavior.

Do not place domain decisions in output, logging, or environment helpers.

## Placement and tests

Use colocated `#[cfg(test)]` tests for pure parsing, matching, state transitions, and adapters. Use `tests/` for spawned-binary, filesystem, signal, reload, logging, or socket behavior.

```sh
cargo test <module-or-test-name>
make lint
make tests
make integration  # external behavior changed
```
