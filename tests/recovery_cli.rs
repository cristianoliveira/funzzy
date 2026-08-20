//! Black-box recovery policy proof (TASK-0123).
//!
//! These tests intentionally run without a TTY: prompt mode must default-deny
//! and explicit skip must avoid spawning the configured mutation.

use std::path::{Path, PathBuf};
use std::process::Command;

fn scratch(label: &str) -> PathBuf {
    let path = std::env::temp_dir().join(format!(
        "funzzy-recovery-cli-{}-{}",
        std::process::id(),
        label
    ));
    let _ = std::fs::remove_dir_all(&path);
    std::fs::create_dir_all(&path).expect("create scratch directory");
    path
}

fn config(path: &Path, marker: &Path) {
    config_with_policy(path, marker, "prompt");
}

fn config_with_policy(path: &Path, marker: &Path, policy: &str) {
    let marker = marker.display();
    std::fs::write(
        path,
        format!(
            "execution:\n  recovery_policy: {policy}\njobs:\n  - name: recover @quick\n    run: \"test -f '{marker}'\"\n    recovery: \"touch '{marker}'\"\n    run_on_init: true\n"
        ),
    )
    .expect("write recovery config");
}

fn run(binary: &str, config: &Path, policy: Option<&str>) -> std::process::Output {
    let mut command = Command::new(binary);
    command.args(["-c", config.to_str().unwrap()]);
    if let Some(policy) = policy {
        command.args(["--recovery-policy", policy]);
    }
    command.args(["run", "@quick"]);
    command.output().expect("run binary")
}

#[test]
fn prompt_mode_without_tty_declines_for_funzzy_and_fzz() {
    for binary in [env!("CARGO_BIN_EXE_funzzy"), env!("CARGO_BIN_EXE_fzz")] {
        let root = scratch(if binary.ends_with("fzz") {
            "fzz"
        } else {
            "funzzy"
        });
        let config_path = root.join(".watch.yaml");
        let marker = root.join("recovered");
        config(&config_path, &marker);

        let output = run(binary, &config_path, None);
        assert!(!output.status.success(), "declined recovery must fail");
        assert!(
            !marker.exists(),
            "headless prompt must not mutate workspace"
        );
        assert!(
            String::from_utf8_lossy(&output.stdout).contains("no TTY"),
            "headless decline should explain the safety reason"
        );
    }
}

#[test]
fn configured_skip_never_spawns_recovery() {
    let root = scratch("configured-skip");
    let config_path = root.join(".watch.yaml");
    let marker = root.join("recovered");
    config_with_policy(&config_path, &marker, "skip");

    let output = run(env!("CARGO_BIN_EXE_fzz"), &config_path, None);
    assert!(!output.status.success());
    assert!(!marker.exists());
    assert!(String::from_utf8_lossy(&output.stdout).contains("recovery_policy: skip"));
}

#[test]
fn explicit_skip_override_never_spawns_recovery() {
    let root = scratch("skip");
    let config_path = root.join(".watch.yaml");
    let marker = root.join("recovered");
    config(&config_path, &marker);

    let output = run(env!("CARGO_BIN_EXE_fzz"), &config_path, Some("skip"));
    assert!(!output.status.success(), "skip must preserve the failure");
    assert!(!marker.exists(), "skip must not mutate workspace");
    assert!(String::from_utf8_lossy(&output.stdout).contains("recovery_policy: skip"));
}
