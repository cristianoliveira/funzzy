//! Black-box exact-generation cancel tests (TASK-0046, contract §10).
//!
//! Proves the wire + CLI surface: cancelling a running generation, no-ops for
//! terminal/unknown generations, stale cancels never affecting a replacement,
//! and `--wait` returning the exact terminal snapshot after cancellation.

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
        let _ = libc_kill(self.child.id() as i32, 15);
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

fn libc_kill(pid: i32, signal: i32) -> std::io::Result<()> {
    let result = unsafe { libc_kill_impl(pid, signal) };
    if result == 0 {
        Ok(())
    } else {
        Err(std::io::Error::last_os_error())
    }
}

unsafe fn libc_kill_impl(pid: i32, signal: i32) -> i32 {
    extern "C" {
        fn kill(pid: i32, sig: i32) -> i32;
    }
    kill(pid, signal)
}

static DIRECTORY_COUNTER: std::sync::atomic::AtomicU32 = std::sync::atomic::AtomicU32::new(0);

fn setup_directory(test_name: &str, config: &str) -> std::path::PathBuf {
    let counter = DIRECTORY_COUNTER.fetch_add(1, std::sync::atomic::Ordering::SeqCst);
    let directory =
        std::env::temp_dir().join(format!("fzzc-{}-{test_name}-{counter}", std::process::id()));
    let _ = std::fs::remove_dir_all(&directory);
    std::fs::create_dir_all(&directory).unwrap();
    std::fs::write(directory.join(".watch.yaml"), config).unwrap();
    std::fs::canonicalize(&directory).expect("canonicalize fixture root")
}

fn start_watcher(directory: &std::path::Path) -> TestProcess {
    let child_log = std::fs::File::create(directory.join("child.err")).unwrap();
    let child = Command::new(env!("CARGO_BIN_EXE_fzz"))
        .current_dir(directory)
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

fn wait_until_status<F: FnMut(&serde_json::Value) -> bool>(socket: &std::path::Path, mut f: F) {
    let mut last_seen = String::new();
    for _ in 0..300 {
        if let Ok(status) = try_status(socket) {
            last_seen = status["result"].to_string();
            if f(status.get("result").unwrap_or(&serde_json::Value::Null)) {
                return;
            }
        }
        std::thread::sleep(Duration::from_millis(100));
    }
    panic!("wait_until_status timed out (last status: {last_seen})");
}

fn reap(output: &mut Output) -> String {
    String::from_utf8_lossy(&output.stdout).to_string()
}

fn combined(output: &Output) -> String {
    format!(
        "{}{}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    )
}

fn scheduled_generation(output: &Output) -> u64 {
    let combined = combined(output);
    combined
        .lines()
        .find_map(|line| line.strip_prefix("scheduled generation: "))
        .unwrap_or_else(|| panic!("scheduled generation line missing in: {combined}"))
        .parse()
        .expect("numeric generation")
}

const LONG_RUNNING: &str = r#"
on:
  socket: sock
tasks:
  - name: long running
    run: "sleep 30"
    change: "*.txt"
"#;

const FAST_ONLY: &str = r#"
on:
  socket: sock
tasks:
  - name: quick
    run: "true"
    change: "*.txt"
"#;

#[test]
fn cancel_running_generation_reports_cancelled() {
    let directory = setup_directory("running", LONG_RUNNING);
    let _watcher = start_watcher(&directory);
    wait_until_socket(&directory);
    let socket = directory.join("sock");

    let emit = run_cli_retry(&directory, &["control", "emit", "a.txt"]);
    let run_id = scheduled_generation(&emit);
    wait_until_status(&socket, |status| {
        status["generation"].as_u64() == Some(run_id) && status["state"].as_str() == Some("running")
    });

    let output = run_cli(
        &directory,
        &["control", "cancel", "--generation", &run_id.to_string()],
    );
    assert!(
        output.status.success(),
        "cancel exits 0: {}",
        combined(&output)
    );
    let stdout = reap(&mut output.clone());
    assert!(stdout.contains("outcome: cancelled"), "stdout: {stdout}");

    wait_until_status(&socket, |status| {
        status["generation"].as_u64() == Some(run_id)
            && status["state"].as_str() == Some("cancelled")
    });
}

#[test]
fn cancel_with_wait_returns_the_exact_terminal_snapshot() {
    let directory = setup_directory("wait", LONG_RUNNING);
    let _watcher = start_watcher(&directory);
    wait_until_socket(&directory);

    let emit = run_cli_retry(&directory, &["control", "emit", "a.txt"]);
    let run_id = scheduled_generation(&emit);

    let output = run_cli(
        &directory,
        &[
            "control",
            "cancel",
            "--generation",
            &run_id.to_string(),
            "--wait",
            "--timeout",
            "15s",
        ],
    );
    assert!(output.status.success(), "cancel --wait exits 0");
    let stdout = combined(&output);
    assert!(stdout.contains("outcome: cancelled"), "stdout: {stdout}");
    assert!(
        stdout.contains("terminal reason: cancelled"),
        "stdout: {stdout}"
    );
    assert!(
        stdout.contains(&format!("generation: {run_id}")),
        "stdout: {stdout}"
    );
}

#[test]
fn cancel_terminal_generation_is_a_noop() {
    let directory = setup_directory("terminal", FAST_ONLY);
    let _watcher = start_watcher(&directory);
    wait_until_socket(&directory);
    let socket = directory.join("sock");

    let emit = run_cli_retry(&directory, &["control", "emit", "a.txt"]);
    let run_id = scheduled_generation(&emit);
    wait_until_status(&socket, |status| {
        status["generation"].as_u64() == Some(run_id) && status["state"].as_str() == Some("passed")
    });

    let output = run_cli(
        &directory,
        &["control", "cancel", "--generation", &run_id.to_string()],
    );
    assert!(output.status.success(), "no-op cancel exits 0");
    let stdout = reap(&mut output.clone());
    assert!(stdout.contains("outcome: not-running"), "stdout: {stdout}");
}

#[test]
fn cancel_unknown_generation_is_a_noop() {
    let directory = setup_directory("unknown", FAST_ONLY);
    let _watcher = start_watcher(&directory);
    wait_until_socket(&directory);

    let output = run_cli(&directory, &["control", "cancel", "--generation", "999"]);
    assert!(output.status.success(), "unknown cancel exits 0");
    let stdout = reap(&mut output.clone());
    assert!(stdout.contains("outcome: not-running"), "stdout: {stdout}");
}

#[test]
fn stale_cancel_does_not_affect_a_replacement_generation() {
    let directory = setup_directory("stale", LONG_RUNNING);
    let _watcher = start_watcher(&directory);
    wait_until_socket(&directory);
    let socket = directory.join("sock");

    let first = run_cli_retry(&directory, &["control", "emit", "a.txt"]);
    let first_id = scheduled_generation(&first);
    wait_until_status(&socket, |status| {
        status["generation"].as_u64() == Some(first_id)
            && status["state"].as_str() == Some("running")
    });

    // A second emit supersedes the first; then a stale cancel for the first
    // must be a no-op that leaves the replacement running.
    let second = run_cli_retry(&directory, &["control", "emit", "b.txt"]);
    let second_id = scheduled_generation(&second);
    assert!(second_id > first_id);

    let output = run_cli(
        &directory,
        &["control", "cancel", "--generation", &first_id.to_string()],
    );
    assert!(output.status.success());
    let stdout = reap(&mut output.clone());
    assert!(stdout.contains("outcome: not-running"), "stdout: {stdout}");

    // The replacement is still the active generation (still running `sleep`).
    wait_until_status(&socket, |status| {
        status["generation"].as_u64() == Some(second_id)
            && status["state"].as_str() == Some("running")
    });
}
