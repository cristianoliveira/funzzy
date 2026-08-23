//! Pseudo-TTY proof for the local recovery approval boundary (TASK-0123).

#![cfg(unix)]

use nix::pty::openpty;
use std::fs::File;
use std::io::{Read, Write};
use std::os::fd::AsRawFd;
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use std::sync::Mutex;

static PTY_LOCK: Mutex<()> = Mutex::new(());

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
    let _pty_guard = PTY_LOCK.lock().expect("pty test lock");
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
    let mut child = command
        .stdin(Stdio::from(child_stdin))
        .stdout(Stdio::from(slave))
        .stderr(Stdio::from(child_stderr))
        .spawn()
        .expect("spawn fzz under pty");

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
                            std::time::Instant::now() + std::time::Duration::from_secs(5);
                        loop {
                            let mut tail = [0_u8; 256];
                            match reader.read(&mut tail) {
                                Ok(size) => output.extend_from_slice(&tail[..size]),
                                Err(error) if error.kind() == std::io::ErrorKind::WouldBlock => {}
                                Err(error) if error.kind() == std::io::ErrorKind::Interrupted => {}
                                Err(error) => panic!("read pty output: {error}"),
                            }
                            if let Some(status) = child.try_wait().expect("poll child") {
                                return (status, String::from_utf8_lossy(&output).into_owned());
                            }
                            assert!(
                                std::time::Instant::now() < deadline,
                                "recovery approval did not time out"
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
    let status = child.wait().expect("wait for fzz");
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
