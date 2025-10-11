## System Design: `--log-file` Flag

### Overview

The `--log-file <file_path>` flag lets Funzzy persist every line that would normally reach STDOUT/STDERR while still streaming it to the console. This document lays out the design required to add the flag to the CLI, integrate logging with the existing watcher pipeline, and provide supporting documentation and tests.

### Goals

- Accept `--log-file <path>` / `-l <path>` for `fzz`, `fzz watch`, and single-command invocations.
- Mirror all Funzzy-emitted output and child command streams to the selected log file **and** the console.
- Avoid breaking existing output formatting, colors, or non-blocking execution.
- Provide deterministic integration coverage under `tests/watching_with_log_file.rs`.

### Non-Goals

- Log rotation or size management.
- Changing colorized output defaults (still controlled by `FUNZZY_COLORED`).
- Capturing third-party processes that alter file descriptors themselves.

### Current State

- CLI parsing lives in `src/main.rs` and `Docopt` does not yet expose a `--log-file` flag.
- Watch commands execute via `cmd::execute` / `cmd::spawn` (`src/cmd.rs`), which forward STDOUT/STDERR directly to the parent terminal.
- Funzzy helper prints (`stdout::info`, `stdout::present_results`, etc.) are thin wrappers around `println!` (`src/stdout.rs`).
- Integration helpers already manage temporary log files through `tests/common/lib.rs::with_output`, ensuring tests run sequentially when they rely on filesystem artifacts.

### Proposed Architecture

#### 1. CLI Argument Plumbing

- Extend `USAGE` in `src/main.rs` to document `-l --log-file <file_path>`.
- Add `pub flag_log_file: Option<String>` to `Args` (`src/main.rs`), and ensure Docopt defaulting handles the absence of the flag.
- Pass the resolved `PathBuf` into `execute_watch_command` and through to both `WatchCommand` and `WatchNonBlockCommand` constructors (`src/main.rs`, `src/cli/watch.rs`, `src/cli/watch_non_block.rs`).

#### 2. Central Logging Facility

- Introduce `src/logging.rs` with a `Logger` struct that owns an optional `File` handle produced by `OpenOptions::new().create(true).write(true).truncate(true)`.
- Expose `logging::init(PathBuf)`, `logging::log_line(&str)`, and `logging::tee(&[u8])` helpers.
- Store the logger in a `once_cell::sync::OnceCell<Logger>` to make it accessible from modules that currently call `println!` or write command output.

#### 3. Updating Funzzy Output Helpers

- Modify `stdout::info`, `stdout::error`, `stdout::verbose`, and `stdout::present_results` (`src/stdout.rs`) to invoke `logging::log_line` alongside `println!`.
- Audit remaining direct `println!` calls (e.g., blank lines in `src/cmd.rs`, watcher diagnostics) and swap them for lightweight helpers such as `stdout::plain(&str)` so they participate in logging.

#### 4. Capturing Child Command Streams

- Update `cmd::execute` and `cmd::spawn` (`src/cmd.rs`) to:
  - Spawn processes with `stdout(Stdio::piped())` and `stderr(Stdio::piped())`.
  - Spawn async reader threads that forward each chunk to the terminal (preserving order per stream) **and** invoke `logging::tee`.
  - Preserve existing blocking semantics (wait for status, return `Child`).
- Ensure non-blocking watch mode (`src/cli/watch_non_block.rs`) continues to kill child processes correctly; the logger should close gracefully when handles drop.

#### 5. Error Handling & UX

- If the provided path’s parent directory is missing, emit a clear error via `stdout::failure` before starting watchers (`src/main.rs`).
- When file creation fails (permission denied, read-only FS), surface the error and exit with status 1.
- Document that repeated runs truncate the file (already noted in `docs/FLAG_LOG_FILE.md`).

### Testing Strategy

- Add new integration coverage in `tests/watching_with_log_file.rs` to assert:
  - Log file creation and contents mirror console output for successful tasks.
  - Errors and non-zero exits are captured.
  - Non-blocking mode continues to write to the file without interleaving issues.
- Reuse `tests/common/lib.rs::with_output` to manage fixture files and enforce serialized execution when touching the filesystem.

### Documentation Updates

- Keep `docs/FLAG_LOG_FILE.md` aligned with the CLI syntax and behavior.
- Update `README.md` usage snippets if they surface CLI flags.

### Implementation Plan (High-Level)

1. **Argument & Usage Updates**
   - `src/main.rs`, `src/cli/watch.rs`, `src/cli/watch_non_block.rs`
2. **Logging Module Introduction**
   - New file: `src/logging.rs`
   - Wiring into `src/stdout.rs` and `src/cmd.rs`
3. **Command Execution Refactor**
   - `src/cmd.rs` for teeing STDOUT/STDERR
   - Auxiliary helpers in `stdout` to avoid raw `println!`
4. **Watcher Integration**
   - Ensure `WatchCommand` / `WatchNonBlockCommand` propagate log file context when spinning up tasks.
5. **Tests & Fixtures**
   - New `tests/watching_with_log_file.rs`
   - Possible updates to `tests/common/lib.rs` if shared helpers are required.
6. **Docs & Changelog**
   - `docs/FLAG_LOG_FILE.md`, `README.md`, and release notes as needed.

### Risks & Mitigations

- **Output Ordering:** Separate STDOUT/STDERR threads may race; mitigate by prefixing entries with stream tags or by using a shared queue if ordering becomes an issue.
- **Performance Overhead:** Tee operations add IO; keep writes buffered and avoid holding global locks longer than necessary.
- **Cross-Platform Behavior:** Ensure file handles close cleanly on Ctrl+C; tie logger lifetime to main thread drop or use `CtrlC` handler if needed.

### Open Questions

- Should Funzzy log its own verbose separator lines exactly as printed, or should the log omit ANSI codes when `FUNZZY_COLORED` is enabled? (Current plan preserves existing behavior.)
- Do we need to support appending instead of truncating on successive runs?

