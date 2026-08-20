# Rust application modules

## Execution shape

```text
main.rs
  -> config.rs / rules.rs / template.rs / yaml.rs
  -> app.rs -> reload_session.rs -> reload.rs / reload_coordinator.rs
  -> watch_loop.rs (blocking | non-block strategies)
  -> watches.rs <- watcher.rs
  -> cli/watch.rs | cli/watch_non_block.rs
  -> cmd.rs | workers.rs -> watcher_state.rs
  -> control.rs / awaiting.rs / snapshot.rs
```

`main.rs` is composition root: parse CLI, choose config or stdin input, select target, choose blocking/non-block execution, and wire reload/logging behavior. Keep concrete wiring here; move reusable policy to owning module.

## Main modules

### Workflow tasks — `rules.rs`, `config.rs`, `template.rs`, `yaml.rs`

`rules.rs` owns the user-facing task model (`Rules` and `OutputPolicy`): matching semantics, glob validation, and task-level presentation. `config.rs` owns YAML parsing, legacy list and grouped `on`/`tasks` compatibility, nested groups, merging, and file loading. `template.rs` is pure command template expansion with no YAML or stdout side effects — unknown variables are returned to callers, who decide how to present them. `yaml.rs` holds low-level YAML extraction helpers.

- Keep YAML shape handling in parser functions (`config.rs`) and task invariants on `Rules`.
- The task model does not retain raw YAML; verbose render (`config::rule_as_yaml`) is reconstructed from model fields.
- Preserve relative versus absolute glob behavior.
- Test malformed and valid forms, including legacy lists and grouped `on`/`tasks`.

### Watch orchestration — `watch_loop.rs`, `watches.rs`, `watcher.rs`

`watch_loop.rs` owns the single application flow: filesystem readiness, init/change event-to-run conversion, and the injected executor strategies (`BlockingStrategy`, `NonBlockStrategy`). CLI watch commands stay thin: build a strategy and call `watch_loop`. `watches.rs` maps tasks to concrete watch roots and selects tasks for changed paths. `watcher.rs` is only filesystem-event adapter.

- Ignore match wins before change match.
- Normalize both project-relative and absolute paths explicitly.
- Register every watch before signaling readiness; initialization must not create event-loss gap.
- Keep `notify` details out of rule policy.
- Control socket publishes through the `NonBlockStrategy` run contract, never worker internals.

### Run lifecycle — `workers.rs`, `cmd.rs`

`Worker` schedules sequential commands, assigns generation IDs, supports cancellation, and emits `WorkerEvent`. `cmd.rs` owns child-process wrapper and shell execution.

- Preserve sequential command order and fail-fast semantics.
- A newer non-block run cancels active child before replacement work.
- Treat generation as external run identity; never reuse or decrement it.
- Keep process errors visible as failures, not silent skips.

### Observable state and control API — `watcher_state.rs`, `awaiting.rs`, `snapshot.rs`, `control.rs`

`watcher_state.rs` projects executor events into one coherent latest-generation `WatcherState`. `awaiting.rs` owns exact-generation waits and freshness, `snapshot.rs` owns correlated subscription snapshots, and `control.rs` exposes those capabilities as JSON-RPC 2.0 over permission-restricted Unix socket.

- Keep transport validation in `control.rs`; state transitions must remain in `watcher_state.rs`.
- Compose optional protocol capabilities through one named `ControlApi`, then call `ControlServer::bind`; do not restore positional `start_with_*` constructors.
- Notifications have no response; requests preserve caller ID.
- `run` returns scheduled generation so clients can await exact work.
- Wire-format changes require matching Pi watcher contract changes and integration tests.

### Live configuration reload — `reload_session.rs`, `reload.rs`, `reload_coordinator.rs`

`reload_session.rs` owns config-file event filtering and long-running reload lifecycle. `reload.rs` validates and classifies candidate revisions. `reload_coordinator.rs` owns prepare/commit/retire transaction and socket/root handoff.

- `app.rs` wires, starts, and joins reload session; do not move event loop back into composition root.
- Preserve prepare → commit → retire and bind-new-before-retire-old ordering.
- Pure path/event selection belongs in `reload_session.rs`; candidate policy belongs in `reload.rs`.

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
