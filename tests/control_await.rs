//! Black-box atomic-await tests (TASK-0044, contract §4/§3).
//!
//! Proves the wire + CLI surface: unambiguous modes, already-terminal and
//! future completion, no-generation-yet, superseded during wait, watcher
//! disconnect/restart, multiple waiters, timeout boundary, `run/emit --wait`
//! composition, and freshness classification in the returned observation.

#![cfg(all(feature = "test-integration", unix))]

use std::io::{BufRead, BufReader, Write};
use std::os::unix::net::UnixStream;
use std::process::{Child, Command, Output, Stdio};
use std::time::Duration;

struct TestProcess {
    child: Child,
    directory: std::path::PathBuf,
}

impl TestProcess {
    fn kill(&mut self) {
        let _ = self.child.kill();
        let _ = self.child.wait();
    }
}

impl Drop for TestProcess {
    fn drop(&mut self) {
        // Graceful SIGTERM first: the watcher's shutdown handler reaps its
        // task process groups, so long-running sleep children never pile up
        // across tests. SIGKILL is the fallback for a stuck watcher.
        let _ = unsafe { libc_kill(self.child.id() as i32, 15) };
        for _ in 0..50 {
            if self.child.try_wait().ok().flatten().is_some() {
                break;
            }
            std::thread::sleep(Duration::from_millis(100));
        }
        let _ = self.child.kill();
        let _ = self.child.wait();
        let _ = std::fs::remove_dir_all(&self.directory);
    }
}

/// Minimal libc kill wrapper (signal 15 = SIGTERM) without extra deps.
fn libc_kill(pid: i32, signal: i32) -> std::io::Result<()> {
    let result = unsafe { libc_kill_impl(pid, signal) };
    if result == 0 {
        Ok(())
    } else {
        Err(std::io::Error::last_os_error())
    }
}

unsafe fn libc_kill_impl(pid: i32, signal: i32) -> i32 {
    // FFI to kill(2) via the standard library's process primitives.
    extern "C" {
        fn kill(pid: i32, sig: i32) -> i32;
    }
    kill(pid, signal)
}

static DIRECTORY_COUNTER: std::sync::atomic::AtomicU32 = std::sync::atomic::AtomicU32::new(0);

fn setup_directory(test_name: &str, config: &str) -> std::path::PathBuf {
    let counter = DIRECTORY_COUNTER.fetch_add(1, std::sync::atomic::Ordering::SeqCst);
    let directory =
        std::env::temp_dir().join(format!("fzza-{}-{test_name}-{counter}", std::process::id()));
    let _ = std::fs::remove_dir_all(&directory);
    std::fs::create_dir_all(&directory).unwrap();
    std::fs::write(directory.join(".watch.yaml"), config).unwrap();
    std::fs::canonicalize(&directory).expect("canonicalize fixture root")
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
    for _ in 0..150 {
        if UnixStream::connect(&socket_path).is_ok() {
            return;
        }
        std::thread::sleep(Duration::from_millis(100));
    }
    panic!(
        "control socket never connectable at {}",
        socket_path.display()
    );
}

fn run_cli(directory: &std::path::Path, args: &[&str]) -> Output {
    Command::new(env!("CARGO_BIN_EXE_fzz"))
        .current_dir(directory)
        .args(args)
        .output()
        .expect("fzz control client should run")
}

fn spawn_cli(directory: &std::path::Path, args: &[&str]) -> Child {
    Command::new(env!("CARGO_BIN_EXE_fzz"))
        .current_dir(directory)
        .args(args)
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .expect("fzz control client should spawn")
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

fn try_status(socket_path: &std::path::Path) -> Result<serde_json::Value, String> {
    let mut stream = match UnixStream::connect(socket_path) {
        Ok(stream) => stream,
        Err(err) => return Err(err.to_string()),
    };
    writeln!(
        stream,
        r#"{{"jsonrpc":"2.0","id":"status","method":"status"}}"#
    )
    .map_err(|err| err.to_string())?;
    let mut line = String::new();
    BufReader::new(&mut stream)
        .read_line(&mut line)
        .map_err(|err| err.to_string())?;
    serde_json::from_str(&line).map_err(|err| err.to_string())
}

fn run_cli_retry(directory: &std::path::Path, args: &[&str]) -> Output {
    let mut last = run_cli(directory, args);
    for _ in 0..2 {
        if last.status.success() {
            return last;
        }
        std::thread::sleep(Duration::from_millis(500));
        last = run_cli(directory, args);
    }
    last
}

fn wait_until<F: FnMut() -> bool>(mut condition: F) {
    for _ in 0..300 {
        if condition() {
            return;
        }
        std::thread::sleep(Duration::from_millis(100));
    }
    panic!("wait_until timed out");
}

fn wait_until_status<F: FnMut(&serde_json::Value) -> bool>(socket: &std::path::Path, mut f: F) {
    let mut last_error = String::new();
    let mut last_seen = String::new();
    for _ in 0..300 {
        match try_status(socket) {
            Ok(status) => {
                last_seen = status["result"].to_string();
                if f(status.get("result").unwrap_or(&serde_json::Value::Null)) {
                    return;
                }
            }
            // A transient connect failure (watcher under heavy load) is not a
            // test failure: keep polling until the bound expires.
            Err(err) => last_error = err,
        }
        std::thread::sleep(Duration::from_millis(100));
    }
    let parent = socket.parent().unwrap_or(std::path::Path::new("."));
    let err_log = std::fs::read_to_string(parent.join("child.err"))
        .unwrap_or_else(|_| "(no child.err)".to_string());
    let out_log = std::fs::read_to_string(parent.join("child.out"))
        .unwrap_or_else(|_| "(no child.out)".to_string());
    panic!(
        "wait_until_status timed out (last error: {last_error}, last status: {last_seen})\nwatcher stderr:\n{err_log}\nwatcher stdout:\n{out_log}"
    );
}

fn reap(output: &mut Output) -> String {
    String::from_utf8_lossy(&output.stdout).to_string()
}

/// Parses `scheduled generation: N` from a control CLI output; the panic
/// includes the full output so a flaky failure shows the real server error.
fn scheduled_generation(output: &Output) -> u64 {
    let stdout = String::from_utf8_lossy(&output.stdout);
    let combined = format!("{}{}", stdout, String::from_utf8_lossy(&output.stderr));
    combined
        .lines()
        .find_map(|line| line.strip_prefix("scheduled generation: "))
        .expect(&format!("scheduled generation line missing in: {combined}"))
        .parse()
        .expect("numeric generation")
}

const INIT_FAST: &str = r#"
on:
  socket: sock
tasks:
  - name: init task
    run: "true"
    change: "*.txt"
    run_on_init: true
"#;

const NO_INIT: &str = r#"
on:
  socket: sock
tasks:
  - name: long running
    run: "sleep 6"
    change: "*.txt"
  - name: quick
    run: "true"
    change: "*.txt"
"#;

/// Fast-only matching config: any `.txt` change runs an instant task, so a
/// scheduled generation reaches terminal immediately.
const FAST_ONLY: &str = r#"
on:
  socket: sock
tasks:
  - name: quick
    run: "true"
    change: "*.txt"
"#;

#[test]
fn await_already_terminal_generation_returns_immediately() {
    let directory = setup_directory("already-terminal", INIT_FAST);
    let _watcher = start_watcher(&directory);
    wait_until_socket(&directory);
    let socket = directory.join("sock");
    wait_until_status(&socket, |status| {
        status["generation"].as_u64() == Some(1) && status["state"].as_str() == Some("passed")
    });

    let output = run_cli(
        &directory,
        &["control", "await", "--generation", "1", "--timeout", "5s"],
    );
    assert!(
        output.status.success(),
        "already-terminal await exits 0: {}",
        reap(&mut output.clone())
    );
    let stdout = reap(&mut output.clone());
    assert!(
        stdout.contains("terminal reason: passed"),
        "stdout: {stdout}"
    );
    assert!(stdout.contains("freshness: current"), "stdout: {stdout}");
    assert!(stdout.contains("generation: 1"), "stdout: {stdout}");
}

#[test]
fn await_future_completion_blocks_then_returns() {
    let directory = setup_directory("future", INIT_FAST);
    let _watcher = start_watcher(&directory);
    wait_until_socket(&directory);
    let socket = directory.join("sock");
    wait_until_status(&socket, |status| status["generation"].as_u64() == Some(1));

    // A second generation is triggered by a file write; awaiting it must
    // block until it reaches terminal, then return passed with exit 0.
    std::fs::write(directory.join("notes.txt"), "x").unwrap();
    wait_until_status(&socket, |status| status["generation"].as_u64() == Some(2));

    let output = run_cli(
        &directory,
        &["control", "await", "--generation", "2", "--timeout", "10s"],
    );
    assert!(output.status.success());
    let stdout = reap(&mut output.clone());
    assert!(
        stdout.contains("terminal reason: passed"),
        "stdout: {stdout}"
    );
    assert!(stdout.contains("generation: 2"), "stdout: {stdout}");
}

#[test]
fn await_no_generation_yet_times_out_with_latest_snapshot() {
    let directory = setup_directory("no-gen", NO_INIT);
    let _watcher = start_watcher(&directory);
    wait_until_socket(&directory);

    let output = run_cli(
        &directory,
        &["control", "await", "--after", "0", "--timeout", "800ms"],
    );
    assert_eq!(output.status.code(), Some(1));
    let stdout = reap(&mut output.clone());
    assert!(
        stdout.contains("terminal reason: timeout"),
        "stdout: {stdout}"
    );
    assert!(stdout.contains("generation: 0"), "stdout: {stdout}");
    assert!(stdout.contains("state: idle"), "stdout: {stdout}");
}

#[test]
fn await_superseded_generation_returns_superseded() {
    let directory = setup_directory("superseded", NO_INIT);
    let _watcher = start_watcher(&directory);
    wait_until_socket(&directory);
    let socket = directory.join("sock");

    // Generation 1 runs for 30s; awaiting it while a new batch arrives must
    // return superseded with the newer snapshot, exit 1.
    let first = run_cli_retry(&directory, &["control", "emit", "a.txt"]);
    if !first.status.success() {
        let watcher_log = std::fs::read_to_string(directory.join("child.err"))
            .unwrap_or_else(|_| "(no watcher stderr)".to_string());
        panic!(
            "first emit failed: {}; watcher stderr:\n{}",
            String::from_utf8_lossy(&first.stdout),
            watcher_log
        );
    }
    let run_one = scheduled_generation(&first);
    wait_until_status(&socket, |status| {
        status["generation"].as_u64() == Some(run_one)
            && status["state"].as_str() == Some("running")
    });

    let mut waiter = spawn_cli(
        &directory,
        &[
            "control",
            "await",
            "--generation",
            &run_one.to_string(),
            "--timeout",
            "20s",
        ],
    );
    std::thread::sleep(Duration::from_millis(1200));
    let second = run_cli_retry(&directory, &["control", "emit", "b.txt"]);
    let run_two = scheduled_generation(&second);
    assert!(run_two > run_one);

    let output = waiter.wait_with_output().expect("waiter finished");
    assert_eq!(output.status.code(), Some(1), "superseded await exits 1");
    let stdout = String::from_utf8_lossy(&output.stdout);
    assert!(
        stdout.contains("terminal reason: superseded"),
        "stdout: {stdout}"
    );
    assert!(
        stdout.contains(&format!("generation: {run_two}")),
        "stdout: {stdout}"
    );
}

#[test]
fn await_watcher_disconnect_reports_disconnected() {
    let directory = setup_directory("disconnect", NO_INIT);
    let mut watcher = start_watcher(&directory);
    wait_until_socket(&directory);

    let mut waiter = spawn_cli(
        &directory,
        &["control", "await", "--after", "0", "--timeout", "60s"],
    );
    std::thread::sleep(Duration::from_millis(1200));
    watcher.kill();

    let output = waiter.wait_with_output().expect("waiter finished");
    assert_eq!(output.status.code(), Some(1), "disconnected await exits 1");
    let combined = format!(
        "{}{}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    );
    assert!(combined.contains("disconnected"), "output: {combined}");
}

#[test]
fn await_watcher_restart_reports_restarted() {
    let directory = setup_directory("restart", INIT_FAST);
    let mut watcher = start_watcher(&directory);
    wait_until_socket(&directory);
    let socket = directory.join("sock");
    wait_until_status(&socket, |status| status["generation"].as_u64() == Some(1));

    let mut waiter = spawn_cli(
        &directory,
        &["control", "await", "--after", "1", "--timeout", "60s"],
    );
    std::thread::sleep(Duration::from_millis(1200));

    // Replace the instance at the same socket path before the client's
    // re-negotiation window closes.
    watcher.kill();
    let _ = std::fs::remove_file(&socket);
    let replacement = start_watcher(&directory);
    wait_until_socket(&directory);

    let output = waiter.wait_with_output().expect("waiter finished");
    assert_eq!(output.status.code(), Some(1), "restarted await exits 1");
    let combined = format!(
        "{}{}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    );
    assert!(combined.contains("restarted"), "output: {combined}");
    let _ = replacement;
}

#[test]
fn multiple_waiters_all_return_on_one_terminal_event() {
    let directory = setup_directory("multi-waiter", FAST_ONLY);
    let _watcher = start_watcher(&directory);
    wait_until_socket(&directory);

    let mut first = spawn_cli(
        &directory,
        &["control", "await", "--after", "0", "--timeout", "20s"],
    );
    let mut second = spawn_cli(
        &directory,
        &["control", "await", "--after", "0", "--timeout", "20s"],
    );
    std::thread::sleep(Duration::from_millis(800));

    let trigger = run_cli(&directory, &["control", "emit", "x.txt"]);
    assert!(trigger.status.success());

    for mut waiter in [first, second] {
        let output = waiter.wait_with_output().expect("waiter finished");
        assert!(output.status.success(), "waiter must exit 0");
        let stdout = String::from_utf8_lossy(&output.stdout);
        assert!(
            stdout.contains("terminal reason: passed"),
            "stdout: {stdout}"
        );
    }
}

#[test]
fn await_timeout_performs_no_cancellation() {
    let directory = setup_directory("timeout", NO_INIT);
    let _watcher = start_watcher(&directory);
    wait_until_socket(&directory);
    let socket = directory.join("sock");

    // A generation runs long; a short await on a future generation times out
    // and must NOT cancel the running work.
    let emit = run_cli_retry(&directory, &["control", "emit", "a.txt"]);
    let run_id = scheduled_generation(&emit);
    wait_until_status(&socket, |status| {
        status["generation"].as_u64() == Some(run_id) && status["state"].as_str() == Some("running")
    });

    let output = run_cli(
        &directory,
        &[
            "control",
            "await",
            "--generation",
            "999",
            "--timeout",
            "500ms",
        ],
    );
    assert_eq!(output.status.code(), Some(1));
    let stdout = reap(&mut output.clone());
    assert!(
        stdout.contains("terminal reason: timeout"),
        "stdout: {stdout}"
    );

    // The running generation is untouched.
    wait_until_status(&socket, |status| {
        status["generation"].as_u64() == Some(run_id) && status["state"].as_str() == Some("running")
    });
}

#[test]
fn control_run_wait_returns_one_observation() {
    let directory = setup_directory("run-wait", INIT_FAST);
    let _watcher = start_watcher(&directory);
    wait_until_socket(&directory);
    let socket = directory.join("sock");
    wait_until_status(&socket, |status| status["generation"].as_u64() == Some(1));

    let output = run_cli(
        &directory,
        &["control", "run", "init task", "--wait", "--timeout", "10s"],
    );
    assert!(output.status.success(), "run --wait passed exits 0");
    let stdout = reap(&mut output.clone());
    assert!(stdout.contains("scheduled generation:"), "stdout: {stdout}");
    assert!(
        stdout.contains("terminal reason: passed"),
        "stdout: {stdout}"
    );
}

#[test]
fn control_run_wait_reports_failed_workflow() {
    let directory = setup_directory(
        "run-wait-failed",
        r#"
on:
  socket: sock
tasks:
  - name: failing target
    run: "false"
    change: "*.txt"
"#,
    );
    let _watcher = start_watcher(&directory);
    wait_until_socket(&directory);

    let output = run_cli(
        &directory,
        &[
            "control",
            "run",
            "failing target",
            "--wait",
            "--timeout",
            "10s",
        ],
    );
    assert_eq!(output.status.code(), Some(1), "failed workflow exits 1");
    let stdout = reap(&mut output.clone());
    assert!(
        stdout.contains("terminal reason: failed"),
        "stdout: {stdout}"
    );
}

#[test]
fn control_emit_wait_returns_one_observation() {
    let directory = setup_directory("emit-wait", FAST_ONLY);
    let _watcher = start_watcher(&directory);
    wait_until_socket(&directory);

    let output = run_cli(
        &directory,
        &["control", "emit", "notes.txt", "--wait", "--timeout", "10s"],
    );
    assert!(output.status.success(), "emit --wait passed exits 0");
    let stdout = reap(&mut output.clone());
    assert!(stdout.contains("outcome: scheduled"), "stdout: {stdout}");
    assert!(
        stdout.contains("terminal reason: passed"),
        "stdout: {stdout}"
    );
}

#[test]
fn control_emit_wait_noop_stays_explicit_noop() {
    let directory = setup_directory("emit-wait-noop", NO_INIT);
    let _watcher = start_watcher(&directory);
    wait_until_socket(&directory);

    let output = run_cli(
        &directory,
        &[
            "control",
            "emit",
            "src/main.rs",
            "--wait",
            "--timeout",
            "5s",
        ],
    );
    assert!(output.status.success(), "no-op emit --wait exits 0");
    let stdout = reap(&mut output.clone());
    assert!(stdout.contains("outcome: unmatched"), "stdout: {stdout}");
    assert!(
        !stdout.contains("terminal reason"),
        "no observation for a no-op: {stdout}"
    );
}

#[test]
fn control_await_usage_errors_exit_two() {
    let directory = setup_directory("usage", INIT_FAST);
    let _watcher = start_watcher(&directory);
    wait_until_socket(&directory);

    let missing_timeout = run_cli(&directory, &["control", "await", "--after", "1"]);
    assert_eq!(missing_timeout.status.code(), Some(2));

    let both_modes = run_cli(
        &directory,
        &[
            "control",
            "await",
            "--after",
            "1",
            "--generation",
            "2",
            "--timeout",
            "1s",
        ],
    );
    assert_eq!(both_modes.status.code(), Some(2));

    let bad_duration = run_cli(
        &directory,
        &["control", "await", "--after", "1", "--timeout", "1h"],
    );
    assert_eq!(bad_duration.status.code(), Some(2));
}
