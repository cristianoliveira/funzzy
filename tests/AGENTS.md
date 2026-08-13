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

Some tests compile in default runs but skip filesystem behavior unless feature is enabled. Do not accept a default `cargo test` pass as proof of integration behavior.

## Verification

```sh
cargo test --test <test_file> --features test-integration -- --nocapture
make integration
make integration-e2e  # only when real filesystem end-to-end coverage is required
```

On failure, inspect unique `.log` output left by test setup before changing timing. Control-socket tests are Unix-only and must remain guarded accordingly.
