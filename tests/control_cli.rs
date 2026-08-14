//! Black-box tests for the `fzz control` CLI client (TASK-0021).
//!
//! Spawns a real watcher over its control socket and drives the `fzz`
//! binary as a client: status, list, run scheduling, socket resolution
//! precedence, unavailable socket, and server-error surfacing.

#![cfg(all(feature = "test-integration", unix))]

use std::io::{BufRead, BufReader, Write};
use std::os::unix::net::UnixStream;
use std::process::{Child, Command, Output, Stdio};
use std::time::Duration;

struct TestProcess {
    child: Child,
    directory: std::path::PathBuf,
}

impl Drop for TestProcess {
    fn drop(&mut self) {
        let _ = self.child.kill();
        let _ = self.child.wait();
        let _ = std::fs::remove_dir_all(&self.directory);
    }
}

static DIRECTORY_COUNTER: std::sync::atomic::AtomicU32 = std::sync::atomic::AtomicU32::new(0);

fn setup_watcher_directory(test_name: &str) -> std::path::PathBuf {
    let counter = DIRECTORY_COUNTER.fetch_add(1, std::sync::atomic::Ordering::SeqCst);
    let directory =
        std::env::temp_dir().join(format!("fzzc-{}-{test_name}-{counter}", std::process::id()));
    let _ = std::fs::remove_dir_all(&directory);
    std::fs::create_dir_all(&directory).unwrap();
    std::fs::write(
        directory.join(".watch.yaml"),
        r#"
on:
  socket: sock
tasks:
  - name: fast tests @agent-fast
    run: "true"
    change: "*.txt"
    ignore: "generated/**"
    run_on_init: true
  - name: full tests @agent-final
    run: 'test -z "{{filepath}}"'
    change: ".funzzy-final-never"
"#,
    )
    .unwrap();
    directory
}

fn start_watcher(directory: &std::path::Path) -> TestProcess {
    let child_log = std::fs::File::create(directory.join("child.err")).unwrap();
    let child = Command::new(env!("CARGO_BIN_EXE_fzz"))
        .current_dir(directory)
        // Isolate from the ambient environment: fail-fast and non-block flags
        // must come from the test's own config, not the developer's shell.
        .env_remove("FUNZZY_BAIL")
        .env_remove("FUNZZY_NON_BLOCK")
        .stdout(Stdio::from(child_log.try_clone().unwrap()))
        .stderr(Stdio::from(child_log))
        .spawn()
        .unwrap();
    TestProcess {
        child,
        directory: directory.to_path_buf(),
    }
}

fn wait_until_socket(directory: &std::path::Path) {
    let socket_path = directory.join("sock");
    for _ in 0..100 {
        if socket_path.exists() {
            return;
        }
        std::thread::sleep(Duration::from_millis(100));
    }
    panic!("control socket never appeared at {}", socket_path.display());
}

fn run_cli(directory: &std::path::Path, args: &[&str]) -> Output {
    Command::new(env!("CARGO_BIN_EXE_fzz"))
        .current_dir(directory)
        .args(args)
        .output()
        .expect("fzz control client should run")
}

fn raw_status(socket_path: &std::path::Path) -> serde_json::Value {
    let mut stream = UnixStream::connect(socket_path).expect("connect control socket");
    writeln!(
        stream,
        r#"{{"jsonrpc":"2.0","id":"status","method":"status"}}"#
    )
    .unwrap();
    let mut line = String::new();
    BufReader::new(&stream).read_line(&mut line).unwrap();
    serde_json::from_str(&line).unwrap()
}

fn wait_until<F: FnMut() -> bool>(mut condition: F) {
    for _ in 0..100 {
        if condition() {
            return;
        }
        std::thread::sleep(Duration::from_millis(100));
    }
    panic!("wait_until timed out");
}

#[test]
fn control_status_prints_compact_state() {
    let directory = setup_watcher_directory("status");
    let _watcher = start_watcher(&directory);
    wait_until_socket(&directory);

    let output = run_cli(&directory, &["control", "status"]);
    assert!(
        output.status.success(),
        "status must exit 0: {}",
        String::from_utf8_lossy(&output.stdout)
    );
    let stdout = String::from_utf8_lossy(&output.stdout);
    assert!(stdout.contains("state:"), "stdout: {}", stdout);
    assert!(stdout.contains("generation:"), "stdout: {}", stdout);
    // Raw command output must stay on the watcher side; the client prints
    // correlation fields, not child output.
    assert!(!stdout.contains("test -z"), "stdout: {}", stdout);
}

#[test]
fn control_list_prints_remote_targets() {
    let directory = setup_watcher_directory("list");
    let _watcher = start_watcher(&directory);
    wait_until_socket(&directory);

    let output = run_cli(&directory, &["control", "list"]);
    assert!(output.status.success());
    let stdout = String::from_utf8_lossy(&output.stdout);
    assert!(stdout.contains("targets (2):"), "stdout: {}", stdout);
    assert!(
        stdout.contains("fast tests @agent-fast"),
        "stdout: {}",
        stdout
    );
    assert!(
        stdout.contains("full tests @agent-final"),
        "stdout: {}",
        stdout
    );
}

#[test]
fn control_run_returns_scheduled_generation_identity() {
    let directory = setup_watcher_directory("run");
    let _watcher = start_watcher(&directory);
    wait_until_socket(&directory);

    let output = run_cli(&directory, &["control", "run", "@agent-final"]);
    assert!(
        output.status.success(),
        "run must exit 0: {}",
        String::from_utf8_lossy(&output.stdout)
    );
    let stdout = String::from_utf8_lossy(&output.stdout);
    assert!(
        stdout.contains("scheduled generation:"),
        "stdout: {}",
        stdout
    );
    let generation: u64 = stdout
        .trim()
        .strip_prefix("scheduled generation: ")
        .expect("generation line")
        .parse()
        .expect("numeric generation");

    // The returned identity must correlate with the watcher's own run:
    // the generation eventually reaches a terminal state.
    let socket_path = directory.join("sock");
    wait_until(|| {
        let status = raw_status(&socket_path);
        status["result"]["generation"].as_u64() == Some(generation)
            && matches!(
                status["result"]["state"].as_str(),
                Some("passed" | "failed" | "cancelled")
            )
    });
}

#[test]
fn control_status_with_missing_socket_reports_selected_path() {
    let directory = setup_watcher_directory("missing-socket");
    // No watcher is started: the socket must not exist.
    let missing = directory.join("sock");
    let output = run_cli(
        &directory,
        &["control", "--socket", missing.to_str().unwrap(), "status"],
    );
    assert_eq!(output.status.code(), Some(1));
    let stdout = String::from_utf8_lossy(&output.stdout);
    assert!(
        stdout.contains(&missing.to_string_lossy().to_string()),
        "error must report selected path: {}",
        stdout
    );
    assert!(
        stdout.contains("cannot reach control socket"),
        "stdout: {}",
        stdout
    );
}

#[test]
fn control_socket_flag_overrides_config_socket() {
    let directory = setup_watcher_directory("override");
    let _watcher = start_watcher(&directory);
    wait_until_socket(&directory);

    // The watcher listens at sock (from on.socket), but an
    // explicit --socket pointing elsewhere must win and fail actionably.
    let wrong = directory.join("wrong.sock");
    let output = run_cli(
        &directory,
        &["control", "--socket", wrong.to_str().unwrap(), "status"],
    );
    assert_eq!(output.status.code(), Some(1));
    let stdout = String::from_utf8_lossy(&output.stdout);
    assert!(
        stdout.contains("wrong.sock"),
        "override path must be reported: {}",
        stdout
    );
}

#[test]
fn control_run_unknown_target_surfaces_server_error() {
    let directory = setup_watcher_directory("unknown-target");
    let _watcher = start_watcher(&directory);
    wait_until_socket(&directory);

    let output = run_cli(&directory, &["control", "run", "does-not-exist"]);
    assert_eq!(output.status.code(), Some(1));
    let stdout = String::from_utf8_lossy(&output.stdout);
    assert!(
        stdout.contains("No target found for 'does-not-exist'"),
        "server error must surface: {}",
        stdout
    );
}

#[test]
fn control_status_without_any_socket_config_is_actionable() {
    let directory =
        std::env::temp_dir().join(format!("funzzy-control-no-config-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&directory);
    std::fs::create_dir_all(&directory).unwrap();

    let output = run_cli(&directory, &["control", "status"]);
    assert_eq!(output.status.code(), Some(1));
    let stdout = String::from_utf8_lossy(&output.stdout);
    assert!(
        stdout.contains("--socket"),
        "hint must name the override: {}",
        stdout
    );
    assert!(stdout.contains("on.socket"), "stdout: {}", stdout);
    let _ = std::fs::remove_dir_all(&directory);
}

#[test]
fn control_help_lists_nested_subcommands() {
    let output = run_cli(std::path::Path::new("."), &["control", "--help"]);
    assert!(output.status.success());
    let stdout = String::from_utf8_lossy(&output.stdout);
    assert!(stdout.contains("status"), "stdout: {}", stdout);
    assert!(stdout.contains("list"), "stdout: {}", stdout);
    assert!(stdout.contains("run"), "stdout: {}", stdout);
}

#[test]
fn control_without_subcommand_is_a_usage_error() {
    let output = run_cli(std::path::Path::new("."), &["control"]);
    assert_eq!(output.status.code(), Some(2));
}

#[test]
fn control_emit_matched_path_schedules_generation() {
    let directory = setup_watcher_directory("emit-matched");
    let _watcher = start_watcher(&directory);
    wait_until_socket(&directory);

    let output = run_cli(&directory, &["control", "emit", "notes.txt"]);
    assert!(
        output.status.success(),
        "matched emit must exit 0: {}",
        String::from_utf8_lossy(&output.stdout)
    );
    let stdout = String::from_utf8_lossy(&output.stdout);
    assert!(stdout.contains("outcome: scheduled"), "stdout: {}", stdout);
    assert!(
        stdout.contains("fast tests @agent-fast"),
        "matched task must be named: {}",
        stdout
    );
    assert!(
        stdout.contains("scheduled generation:"),
        "stdout: {}",
        stdout
    );

    // The returned identity must correlate with the watcher's own run.
    let generation: u64 = stdout
        .trim()
        .lines()
        .find_map(|line| line.strip_prefix("scheduled generation: "))
        .expect("generation line")
        .parse()
        .expect("numeric generation");
    let socket_path = directory.join("sock");
    wait_until(|| {
        let status = raw_status(&socket_path);
        status["result"]["generation"].as_u64() == Some(generation)
    });
}

#[test]
fn control_emit_nonexistent_path_still_routes_by_pattern() {
    let directory = setup_watcher_directory("emit-nonexistent");
    let _watcher = start_watcher(&directory);
    wait_until_socket(&directory);

    // The path need not exist: deletions and remote logical events remain
    // representable; routing follows change patterns only.
    let output = run_cli(&directory, &["control", "emit", "never-created.txt"]);
    assert!(output.status.success());
    let stdout = String::from_utf8_lossy(&output.stdout);
    assert!(stdout.contains("outcome: scheduled"), "stdout: {}", stdout);
    assert!(
        stdout.contains("scheduled generation:"),
        "stdout: {}",
        stdout
    );
}

#[test]
fn control_emit_ignored_path_is_explicit_noop() {
    let directory = setup_watcher_directory("emit-ignored");
    let _watcher = start_watcher(&directory);
    wait_until_socket(&directory);

    // generated/** matches the ignore pattern, which wins over *.txt.
    let output = run_cli(&directory, &["control", "emit", "generated/out.txt"]);
    assert!(
        output.status.success(),
        "ignored emit must exit 0: {}",
        String::from_utf8_lossy(&output.stdout)
    );
    let stdout = String::from_utf8_lossy(&output.stdout);
    assert!(stdout.contains("outcome: ignored"), "stdout: {}", stdout);
    assert!(stdout.contains("matched: (none)"), "stdout: {}", stdout);
    assert!(
        !stdout.contains("scheduled generation"),
        "no generation must be scheduled: {}",
        stdout
    );
}

#[test]
fn control_emit_unmatched_path_is_explicit_noop() {
    let directory = setup_watcher_directory("emit-unmatched");
    let _watcher = start_watcher(&directory);
    wait_until_socket(&directory);

    let output = run_cli(&directory, &["control", "emit", "src/main.rs"]);
    assert!(
        output.status.success(),
        "unmatched emit must exit 0: {}",
        String::from_utf8_lossy(&output.stdout)
    );
    let stdout = String::from_utf8_lossy(&output.stdout);
    assert!(stdout.contains("outcome: unmatched"), "stdout: {}", stdout);
    assert!(stdout.contains("matched: (none)"), "stdout: {}", stdout);
    assert!(
        !stdout.contains("scheduled generation"),
        "no generation must be scheduled: {}",
        stdout
    );
}

#[test]
fn control_emit_empty_path_is_usage_error() {
    let directory = setup_watcher_directory("emit-empty");
    let _watcher = start_watcher(&directory);
    wait_until_socket(&directory);

    let output = run_cli(&directory, &["control", "emit", ""]);
    assert_eq!(output.status.code(), Some(2));
    let combined = format!(
        "{}{}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    );
    assert!(
        combined.contains("path cannot be empty"),
        "usage error must name the problem: {}",
        combined
    );
}
