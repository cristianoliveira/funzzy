# Rust integration tests

Own black-box behavior of compiled `fzz` binary across CLI, filesystem, processes, logs, config reload, and Unix control socket.

## Test placement

- Add new file when behavior is a distinct user-visible capability.
- Extend existing capability file when changing its accepted/rejected paths.
- Keep pure algorithm tests in owning `src/*.rs` module instead.
- Reuse `tests/common/lib.rs` setup and `tests/common/macros.rs`; do not create a second process/polling harness.

## Determinism

- Give each filesystem test unique output/log paths.
- Use bounded `wait_until!` polling; never fixed sleeps as assertion strategy.
- Ensure spawned children, sockets, files, and temporary directories are cleaned even on failure.
- Disable color and unrelated environment behavior through established test setup.
- Assert observable output/state, not internal implementation.
- Cover success and failure paths. Mocked behavior must not claim to verify real process, filesystem, or socket integration.

### Service PID marker pattern (TASK-0174)

A test that needs a service child's PID must never `unwrap` a PID file the
service itself writes: the file may not exist yet, and `echo $$ > pid` is a
truncate-then-write (a concurrent reader can observe an empty file).

- Service scripts write their PID atomically: `printf '%s\n' "$$" > "pid.tmp.$$" && mv "pid.tmp.$$" pid`, then `touch` their `.started` marker.
- Tests read PIDs only through a bounded wait-for-valid helper
  (`control_await.rs::service_pid`): poll until the file exists AND parses,
  with a load-tolerant deadline and a descriptive panic.
- Gate on the service's own `.started`/ready marker when ordering matters;
  existence of the PID file alone is not a readiness signal for torn writes
  in scripts that have not migrated to atomic writes.
- Wait deadlines are load upper bounds (60s), not sleeps tuned to a machine.

Some tests compile in default runs but skip filesystem behavior unless feature is enabled. Do not accept a default `cargo test` pass as proof of integration behavior.

## Verification

```sh
cargo test --test <test_file> --features test-integration -- --nocapture
make integration
make integration-e2e  # only when real filesystem end-to-end coverage is required
```

On failure, inspect unique `.log` output left by test setup before changing timing. Control-socket tests are Unix-only and must remain guarded accordingly.
