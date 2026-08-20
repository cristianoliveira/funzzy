//! Pseudo-TTY proof for the local recovery approval boundary (TASK-0123).

#![cfg(unix)]

use nix::pty::openpty;
use std::fs::File;
use std::io::{Read, Write};
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};

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
            "execution:\n  recovery_policy: prompt\njobs:\n  - name: recover @quick\n    run: {run:?}\n    recovery: {recovery}\n    run_on_init: true\n"
        ),
    )
    .expect("write recovery config");
}

fn run_with_answer(
    binary: &str,
    config: &Path,
    answer: &[u8],
) -> (std::process::ExitStatus, String) {
    let pty = openpty(None, None).expect("open pty");
    let master = File::from(pty.master);
    let mut writer = master.try_clone().expect("clone pty master");
    let mut reader = master;
    let slave = File::from(pty.slave);
    assert!(nix::unistd::isatty(&slave).expect("check pty slave"));
    let child_stdin = slave.try_clone().expect("clone pty slave");
    let child_stderr = slave.try_clone().expect("clone pty slave for stderr");
    let mut child = Command::new(binary)
        .args(["-c", config.to_str().unwrap(), "run", "@quick"])
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
                    writer.write_all(answer).expect("write approval answer");
                    drop(writer);
                    break;
                }
            }
            Err(error) if error.kind() == std::io::ErrorKind::Interrupted => continue,
            Err(error) => panic!("read pty output: {error}"),
        }
    }
    let status = child.wait().expect("wait for fzz");
    let _ = reader.read_to_end(&mut output);
    (status, String::from_utf8_lossy(&output).into_owned())
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
