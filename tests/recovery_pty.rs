//! Pseudo-TTY proof for the local recovery approval boundary (TASK-0123).

#![cfg(unix)]

use nix::pty::openpty;
use std::fs::{File, OpenOptions};
use std::io::{BufRead, BufReader, Read, Write};
use std::os::fd::AsRawFd;
use std::os::unix::net::UnixStream;
use std::path::{Path, PathBuf};
use std::process::{Child, Command, Stdio};
use std::sync::{Mutex, MutexGuard};

static PTY_LOCK: Mutex<()> = Mutex::new(());

struct ChildCleanup(Option<Child>);

impl Drop for ChildCleanup {
    fn drop(&mut self) {
        if let Some(child) = self.0.as_mut() {
            let _ = child.kill();
            let _ = child.wait();
        }
    }
}

struct PtyGuard {
    _thread: MutexGuard<'static, ()>,
    lock_file: File,
}

impl Drop for PtyGuard {
    fn drop(&mut self) {
        unsafe {
            let _ = nix::libc::flock(self.lock_file.as_raw_fd(), nix::libc::LOCK_UN);
        }
    }
}

fn acquire_pty_guard() -> PtyGuard {
    let thread = PTY_LOCK
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner());
    let path = std::env::temp_dir().join("funzzy-recovery-pty-global.lock");
    let lock_file = OpenOptions::new()
        .create(true)
        .read(true)
        .write(true)
        .open(path)
        .expect("open global pty lock");
    loop {
        let acquired = unsafe {
            nix::libc::flock(
                lock_file.as_raw_fd(),
                nix::libc::LOCK_EX | nix::libc::LOCK_NB,
            )
        };
        if acquired == 0 {
            return PtyGuard {
                _thread: thread,
                lock_file,
            };
        }
        std::thread::sleep(std::time::Duration::from_millis(10));
    }
}

fn scratch(label: &str) -> PathBuf {
    let path = std::env::temp_dir().join(format!(
        "funzzy-recovery-pty-{}-{}",
        std::process::id(),
        label
    ));
    let _ = std::fs::remove_dir_all(&path);
    std::fs::create_dir_all(&path).expect("create scratch directory");
    path
}

fn write_config(path: &Path, marker: &Path) {
    let marker = marker.display();
    write_custom_config(
        path,
        &format!("if test -f '{marker}'; then exit 0; else touch '{marker}.attempt'; exit 1; fi"),
        &format!("\"touch '{marker}'\""),
    );
}

fn write_custom_config(path: &Path, run: &str, recovery: &str) {
    std::fs::write(
        path,
        format!(
            "execution:\n  recovery_policy: prompt\n  recovery_timeout: 60ms\njobs:\n  - name: recover @quick\n    run: {run:?}\n    recovery: {recovery}\n    run_on_init: true\n"
        ),
    )
    .expect("write recovery config");
}

struct PtyWatcher {
    child: Child,
    master: File,
    writer: File,
    root: PathBuf,
}

impl Drop for PtyWatcher {
    fn drop(&mut self) {
        let _ = self.child.kill();
        let _ = self.child.wait();
        let _ = std::fs::remove_dir_all(&self.root);
    }
}

fn start_watcher_with_pty(config: &Path, events: &Path, root: &Path) -> PtyWatcher {
    let pty = openpty(None, None).expect("open watcher pty");
    let master = File::from(pty.master);
    let writer = master.try_clone().expect("clone watcher pty writer");
    let slave = File::from(pty.slave);
    let child_stdin = slave.try_clone().expect("clone watcher stdin");
    let child_stdout = slave.try_clone().expect("clone watcher stdout");
    let child_stderr = slave.try_clone().expect("clone watcher stderr");
    let child = Command::new(env!("CARGO_BIN_EXE_fzz"))
        .current_dir(root)
        .args([
            "-c",
            config.to_str().unwrap(),
            "--events",
            events.to_str().unwrap(),
        ])
        .env("FUNZZY_BAIL", "false")
        .stdin(Stdio::from(child_stdin))
        .stdout(Stdio::from(child_stdout))
        .stderr(Stdio::from(child_stderr))
        .spawn()
        .expect("spawn watcher under pty");
    PtyWatcher {
        child,
        master,
        writer,
        root: root.to_path_buf(),
    }
}

fn wait_for_event(
    events: &Path,
    predicate: impl Fn(&serde_json::Value) -> bool,
) -> serde_json::Value {
    let deadline = std::time::Instant::now() + std::time::Duration::from_secs(5);
    loop {
        if let Ok(text) = std::fs::read_to_string(events) {
            for line in text.lines() {
                let record: serde_json::Value =
                    serde_json::from_str(line).expect("valid event record");
                if predicate(&record) {
                    return record;
                }
            }
        }
        assert!(
            std::time::Instant::now() < deadline,
            "timed out waiting for event; events={}\n{}",
            events.display(),
            std::fs::read_to_string(events)
                .unwrap_or_else(|error| format!("<unreadable: {error}>"))
        );
        std::thread::sleep(std::time::Duration::from_millis(10));
    }
}

fn wait_for_prompt(master: &File) {
    let mut reader = master.try_clone().expect("clone watcher pty reader");
    let fd = reader.as_raw_fd();
    let flags = unsafe { nix::libc::fcntl(fd, nix::libc::F_GETFL) };
    assert!(flags >= 0, "get watcher pty flags");
    assert_eq!(
        unsafe { nix::libc::fcntl(fd, nix::libc::F_SETFL, flags | nix::libc::O_NONBLOCK) },
        0,
        "set watcher pty nonblocking"
    );
    let mut output = Vec::new();
    let mut buffer = [0_u8; 256];
    let deadline = std::time::Instant::now() + std::time::Duration::from_secs(5);
    loop {
        match reader.read(&mut buffer) {
            Ok(size) => output.extend_from_slice(&buffer[..size]),
            Err(error) if error.kind() == std::io::ErrorKind::WouldBlock => {}
            Err(error) if error.kind() == std::io::ErrorKind::Interrupted => {}
            Err(error) => panic!("read watcher pty output: {error}"),
        }
        if output.ends_with(b"[y/N] ") {
            return;
        }
        assert!(
            std::time::Instant::now() < deadline,
            "timed out waiting for approval prompt"
        );
        std::thread::sleep(std::time::Duration::from_millis(10));
    }
}

fn wait_for_socket(path: &Path) {
    let deadline = std::time::Instant::now() + std::time::Duration::from_secs(5);
    loop {
        if UnixStream::connect(path).is_ok() {
            return;
        }
        assert!(
            std::time::Instant::now() < deadline,
            "control socket never connected"
        );
        std::thread::sleep(std::time::Duration::from_millis(10));
    }
}

fn control_call(
    socket: &Path,
    id: &str,
    method: &str,
    params: serde_json::Value,
) -> serde_json::Value {
    let mut stream = UnixStream::connect(socket).expect("connect control socket");
    let request = serde_json::json!({
        "jsonrpc": "2.0",
        "id": id,
        "method": method,
        "params": params,
    });
    writeln!(stream, "{}", request).expect("write control request");
    let mut response = String::new();
    BufReader::new(&mut stream)
        .read_line(&mut response)
        .expect("read control response");
    serde_json::from_str(&response).expect("valid control response")
}

fn run_with_answer(
    binary: &str,
    config: &Path,
    answer: &[u8],
) -> (std::process::ExitStatus, String) {
    run_with_answer_and_events(binary, config, answer, None)
}

fn run_with_answer_and_events(
    binary: &str,
    config: &Path,
    answer: &[u8],
    events: Option<&Path>,
) -> (std::process::ExitStatus, String) {
    let _pty_guard = acquire_pty_guard();
    let pty = openpty(None, None).expect("open pty");
    let master = File::from(pty.master);
    let mut writer = Some(master.try_clone().expect("clone pty master"));
    let mut reader = master;
    let slave = File::from(pty.slave);
    assert!(nix::unistd::isatty(&slave).expect("check pty slave"));
    let child_stdin = slave.try_clone().expect("clone pty slave");
    let child_stderr = slave.try_clone().expect("clone pty slave for stderr");
    let mut command = Command::new(binary);
    command
        .args(["-c", config.to_str().unwrap(), "run", "@quick"])
        .env("FUNZZY_BAIL", "false");
    if let Some(events) = events {
        command.arg("--events").arg(events);
    }
    let child = command
        .stdin(Stdio::from(child_stdin))
        .stdout(Stdio::from(slave))
        .stderr(Stdio::from(child_stderr))
        .spawn()
        .expect("spawn fzz under pty");
    // Keep the child bounded even when a PTY assertion panics; otherwise a
    // stalled approval leaves a live watcher that interferes with later tests.
    let mut child = ChildCleanup(Some(child));

    let mut output = Vec::new();
    let mut byte = [0_u8; 1];
    loop {
        match reader.read(&mut byte) {
            Ok(0) => break,
            Ok(_) => {
                output.push(byte[0]);
                if output.ends_with(b"[y/N] ") {
                    if answer == b"__TIMEOUT__" {
                        writer.take();
                        let fd = reader.as_raw_fd();
                        let flags = unsafe { nix::libc::fcntl(fd, nix::libc::F_GETFL) };
                        assert!(flags >= 0, "get pty flags");
                        let result = unsafe {
                            nix::libc::fcntl(fd, nix::libc::F_SETFL, flags | nix::libc::O_NONBLOCK)
                        };
                        assert_eq!(result, 0, "set pty nonblocking");
                        let deadline =
                            std::time::Instant::now() + std::time::Duration::from_secs(15);
                        loop {
                            let mut tail = [0_u8; 256];
                            match reader.read(&mut tail) {
                                Ok(size) => output.extend_from_slice(&tail[..size]),
                                Err(error) if error.kind() == std::io::ErrorKind::WouldBlock => {}
                                Err(error) if error.kind() == std::io::ErrorKind::Interrupted => {}
                                Err(error) => panic!("read pty output: {error}"),
                            }
                            if let Some(status) = child
                                .0
                                .as_mut()
                                .expect("child remains owned")
                                .try_wait()
                                .expect("poll child")
                            {
                                return (status, String::from_utf8_lossy(&output).into_owned());
                            }
                            assert!(
                                std::time::Instant::now() < deadline,
                                "recovery approval did not time out; output={}",
                                String::from_utf8_lossy(&output)
                            );
                            std::thread::yield_now();
                        }
                    }
                    writer
                        .as_mut()
                        .expect("approval writer is open")
                        .write_all(answer)
                        .expect("write approval answer");
                    writer.take();
                    break;
                }
            }
            Err(error) if error.kind() == std::io::ErrorKind::Interrupted => continue,
            Err(error) => panic!("read pty output: {error}"),
        }
    }
    let status = child
        .0
        .take()
        .expect("child remains owned")
        .wait()
        .expect("wait for fzz");
    (status, String::from_utf8_lossy(&output).into_owned())
}

#[test]
fn approved_recovery_emits_phases_and_runs_only_success_hook_once() {
    let root = scratch("observability");
    let config = root.join(".watch.yaml");
    let events = root.join("events.ndjson");
    let marker = root.join("recovered");
    let hook = root.join("hook.log");
    std::fs::write(
        &config,
        format!(
            "execution:\n  recovery_policy: prompt\nhooks:\n  success: \"printf success >> '{}'\"\n  failure: \"printf failure >> '{}'\"\njobs:\n  - name: recover @quick\n    run: {:?}\n    recovery: \"touch '{}'\"\n    run_on_init: true\n",
            hook.display(),
            hook.display(),
            format!(
                "if test -f '{}'; then exit 0; else exit 1; fi",
                marker.display()
            ),
            marker.display(),
        ),
    )
    .expect("write observability config");
    let (status, output) =
        run_with_answer_and_events(env!("CARGO_BIN_EXE_fzz"), &config, b"yes\n", Some(&events));
    assert!(status.success(), "approved recovery failed: {output}");
    assert_eq!(std::fs::read_to_string(&hook).unwrap(), "success");
    let records: Vec<serde_json::Value> = std::fs::read_to_string(&events)
        .unwrap()
        .lines()
        .map(|line| serde_json::from_str(line).unwrap())
        .collect();
    let phases: Vec<&str> = records
        .iter()
        .filter(|record| record["event"] == "recovery_phase")
        .map(|record| record["phase"].as_str().unwrap())
        .collect();
    assert_eq!(
        phases,
        [
            "original_failed",
            "approval_requested",
            "approval_decided",
            "recovery_started",
            "recovery_finished",
            "verification_started",
            "verification_finished",
        ]
    );
    let terminals: Vec<&serde_json::Value> = records
        .iter()
        .filter(|record| record["event"] == "task_terminal")
        .collect();
    assert_eq!(terminals.len(), 1, "recovery has one final job row");
    assert_eq!(terminals[0]["task"], "recover @quick");
    assert_eq!(terminals[0]["state"], "passed");
    assert!(terminals[0]["durationMs"].is_u64());
    // The PTY helper owns the approval prompt stream; the NDJSON terminal
    // record is the stable cross-surface evidence for this recovered row.
    assert!(records.iter().any(|record| record["event"] == "finished"));
}

#[test]
fn approval_runs_exact_recovery_and_one_verification() {
    for binary in [env!("CARGO_BIN_EXE_funzzy"), env!("CARGO_BIN_EXE_fzz")] {
        let label = if binary.ends_with("fzz") {
            "approved-fzz"
        } else {
            "approved-funzzy"
        };
        let root = scratch(label);
        let config = root.join(".watch.yaml");
        let marker = root.join("recovered");
        write_config(&config, &marker);

        let (status, output) = run_with_answer(binary, &config, b"yes\n");
        assert!(
            status.success(),
            "approved recovery failed: status={status:?} output={output:?} root={root:?}"
        );
        assert!(marker.exists(), "approved recovery must execute");
        assert!(output.contains("Generation 0 failed in jobs: recover @quick"));
        assert!(output.contains("Proposed recoveries (run once, in this order):"));
        assert!(output.contains("touch "));
    }
}

#[test]
fn default_decline_keeps_failure_and_does_not_mutate() {
    let root = scratch("declined");
    let config = root.join(".watch.yaml");
    let marker = root.join("recovered");
    write_config(&config, &marker);

    let (status, output) = run_with_answer(env!("CARGO_BIN_EXE_fzz"), &config, b"n\n");
    assert!(!status.success(), "declined recovery must fail: {output}");
    assert!(!marker.exists(), "declined recovery must not execute");
}

#[test]
fn failed_recovery_does_not_verify_or_retry() {
    let root = scratch("recovery-failure");
    let config = root.join(".watch.yaml");
    let recovered = root.join("recovered");
    let verified = root.join("verified");
    let run = format!(
        "if test -f '{}'; then touch '{}'; exit 0; else exit 1; fi",
        recovered.display(),
        verified.display()
    );
    write_custom_config(&config, &run, "\"false\"");

    let (status, output) = run_with_answer(env!("CARGO_BIN_EXE_fzz"), &config, b"y\n");
    assert!(!status.success(), "failed recovery must fail: {output}");
    assert!(
        !verified.exists(),
        "failed recovery must not trigger verification"
    );
}

#[test]
fn verification_failure_remains_a_final_failure() {
    let root = scratch("verification-failure");
    let config = root.join(".watch.yaml");
    write_custom_config(&config, "\"false\"", "\"true\"");

    let (status, output) = run_with_answer(env!("CARGO_BIN_EXE_fzz"), &config, b"y\n");
    assert!(!status.success(), "failed verification must fail: {output}");
}

#[test]
fn unanswered_recovery_approval_times_out_without_running_recovery() {
    let root = scratch("approval-timeout");
    let config = root.join(".watch.yaml");
    let marker = root.join("recovered");
    let events = root.join("events.ndjson");
    write_custom_config(
        &config,
        "\"false\"",
        &format!("\"touch '{}'\"", marker.display()),
    );

    let (status, output) = run_with_answer_and_events(
        env!("CARGO_BIN_EXE_fzz"),
        &config,
        b"__TIMEOUT__",
        Some(&events),
    );
    assert!(!status.success(), "timed out approval must fail: {output}");
    assert!(!marker.exists(), "timed out approval must not run recovery");
    assert!(
        output.contains("approval timeout"),
        "timeout reason missing: {output}"
    );
    let event_text = std::fs::read_to_string(events).expect("read timeout events");
    assert!(event_text.contains("\"phase\":\"approval_requested\""));
    assert!(event_text.contains("\"phase\":\"approval_decided\""));
    assert!(event_text.contains("approval timeout"));
}

#[test]
fn cancellation_and_supersession_discard_partial_tty_input() {
    let _pty_guard = acquire_pty_guard();
    let root = scratch("cancel-partial-input");
    let config = root.join(".watch.yaml");
    let events = root.join("events.ndjson");
    let marker = root.join("recovered");
    std::fs::write(
        &config,
        format!(
            "on:\n  socket: sock\nexecution:\n  recovery_policy: prompt\n  recovery_timeout: 2s\njobs:\n  - name: recover @quick\n    run: \"false\"\n    recovery: \"touch '{}'\"\n    run_on_init: true\n",
            marker.display()
        ),
    )
    .expect("write cancellation config");

    let mut watcher = start_watcher_with_pty(&config, &events, &root);
    let socket = root.join("sock");
    wait_for_socket(&socket);
    wait_for_event(&events, |event| {
        event["event"] == "recovery_phase"
            && event["runId"] == 1
            && event["phase"] == "approval_requested"
    });
    wait_for_prompt(&watcher.master);

    // Leave a canonical line incomplete for generation 1. Supersession must
    // cancel that approval before it can become input for generation 2.
    watcher
        .writer
        .write_all(b"y")
        .expect("write partial approval");
    let scheduled = control_call(
        &socket,
        "run",
        "run",
        serde_json::json!({"target": "@quick"}),
    );
    assert_eq!(
        scheduled["result"]["runId"], 2,
        "scheduled successor: {scheduled}"
    );
    let cancelled = wait_for_event(&events, |event| {
        event["event"] == "cancelled" && event["runId"] == 1
    });
    assert_eq!(
        cancelled["supersededBy"], 2,
        "supersession relation: {cancelled}"
    );
    wait_for_event(&events, |event| {
        event["event"] == "recovery_phase"
            && event["runId"] == 2
            && event["phase"] == "approval_requested"
    });
    wait_for_prompt(&watcher.master);

    // Complete the stale line only after generation 2 is asking. If stale
    // bytes crossed the boundary this would approve recovery and create the
    // marker; a clean boundary yields EOF/decline and no mutation.
    watcher
        .writer
        .write_all(b"\n")
        .expect("complete stale approval");
    let terminal = wait_for_event(&events, |event| {
        event["event"] == "finished" && event["runId"] == 2
    });
    assert!(!terminal["failures"].as_array().unwrap().is_empty());
    assert!(
        !marker.exists(),
        "stale partial input must not approve successor"
    );
}

#[test]
fn control_status_stays_non_terminal_then_exact_await_reports_timeout() {
    let _pty_guard = acquire_pty_guard();
    let root = scratch("control-timeout");
    let config = root.join(".watch.yaml");
    let events = root.join("events.ndjson");
    let marker = root.join("recovered");
    std::fs::write(
        &config,
        format!(
            "on:\n  socket: sock\nexecution:\n  recovery_policy: prompt\n  recovery_timeout: 100ms\njobs:\n  - name: recover @quick\n    run: \"printf original-timeout >&2; exit 1\"\n    output: show-on-failure\n    recovery: \"touch '{}'\"\n    run_on_init: true\n",
            marker.display()
        ),
    )
    .expect("write control timeout config");

    let watcher = start_watcher_with_pty(&config, &events, &root);
    let socket = root.join("sock");
    wait_for_socket(&socket);
    wait_for_event(&events, |event| {
        event["event"] == "recovery_phase"
            && event["runId"] == 1
            && event["phase"] == "approval_requested"
    });
    wait_for_prompt(&watcher.master);

    let status = control_call(&socket, "status", "status", serde_json::json!({}));
    let status = &status["result"];
    assert_eq!(status["generation"], 1);
    assert_eq!(
        status["state"], "running",
        "approval is non-terminal: {status}"
    );
    assert!(status["tasks"]
        .as_array()
        .is_some_and(|tasks| tasks.is_empty()));

    let awaited = control_call(
        &socket,
        "await",
        "await",
        serde_json::json!({"generation": 1, "timeoutMs": 5000}),
    );
    let result = &awaited["result"];
    assert_eq!(result["terminalReason"], "failed", "exact await: {awaited}");
    assert_eq!(result["snapshot"]["generation"], 1);
    assert_eq!(result["snapshot"]["state"], "failed");
    assert!(result["failureEvidence"]["excerpt"]
        .as_str()
        .is_some_and(|excerpt| excerpt.contains("original-timeout")));
    let timeout = wait_for_event(&events, |event| {
        event["event"] == "recovery_phase"
            && event["runId"] == 1
            && event["phase"] == "approval_decided"
    });
    assert_eq!(
        timeout["outcome"], "approval timeout",
        "timeout evidence: {timeout}"
    );
    assert!(!marker.exists());
    drop(watcher);
}

#[test]
fn multi_command_recovery_runs_in_declared_order_before_verification() {
    let root = scratch("multi-command");
    let config = root.join(".watch.yaml");
    let marker = root.join("recovered");
    let first = root.join("first");
    let run = format!(
        "if test -f '{}'; then exit 0; else exit 1; fi",
        marker.display()
    );
    let recovery = format!(
        "[\"touch {}\", \"touch {}\"]",
        first.display(),
        marker.display()
    );
    write_custom_config(&config, &run, &recovery);

    let (status, output) = run_with_answer(env!("CARGO_BIN_EXE_fzz"), &config, b"y\n");
    assert!(status.success(), "multi-command recovery failed: {output}");
    assert!(first.exists() && marker.exists());
}
