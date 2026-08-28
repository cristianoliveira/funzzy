//! TASK-0137: black-box proof for integration-agnostic command observation
//! through the real `trigger: manual` shape (TASK-0136).
//!
//! A deterministic local blocking script stands in for any external system:
//! the script owns authentication, polling, correlation, retries, and the
//! terminal decision; Funzzy owns only configured command execution and
//! observation. No network, no credentials, no sleep-based assertions.

#![cfg(all(feature = "test-integration", unix))]

use std::io::{BufRead, BufReader};
use std::os::unix::fs::PermissionsExt;
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

/// Root `on.change` is present on purpose: the manual job must not inherit
/// it. The canary job provides the positive boundary — init and filesystem
/// events demonstrably flow while `await-remote` stays silent.
const CONFIG: &str = r#"
on:
  socket: sock
  change: 'src/**'
jobs:
  - name: await-remote
    trigger: manual
    run: ./await-remote.sh
  - name: canary
    run: ./canary.sh
    run_on_init: true
"#;

fn setup_directory(test_name: &str) -> std::path::PathBuf {
    let counter = DIRECTORY_COUNTER.fetch_add(1, std::sync::atomic::Ordering::SeqCst);
    let directory = std::env::temp_dir().join(format!(
        "funzzy-manual-recipe-{}-{test_name}-{counter}",
        std::process::id()
    ));
    let _ = std::fs::remove_dir_all(&directory);
    std::fs::create_dir_all(directory.join("src")).unwrap();
    std::fs::write(directory.join(".watch.yaml"), CONFIG).unwrap();
    std::fs::write(
        directory.join("canary.sh"),
        r#"#!/bin/sh
printf canary >> canary-ran
"#,
    )
    .unwrap();
    std::fs::write(
        directory.join("await-remote.sh"),
        r#"#!/bin/sh
printf started >> script-starts
printf pending-output
while [ ! -f release ]; do sleep 0.02; done
if [ -f fail ]; then
  printf failure-output
  printf failure-error >&2
  exit 7
fi
printf passed-output
"#,
    )
    .unwrap();
    for script in ["canary.sh", "await-remote.sh"] {
        let mut permissions = std::fs::metadata(directory.join(script))
            .unwrap()
            .permissions();
        permissions.set_mode(0o755);
        std::fs::set_permissions(directory.join(script), permissions).unwrap();
    }
    std::fs::canonicalize(directory).unwrap()
}

fn start_watcher(directory: &std::path::Path) -> TestProcess {
    let child_log = std::fs::File::create(directory.join("watcher.log")).unwrap();
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

fn wait_until<F: FnMut() -> bool>(mut condition: F, description: &str) {
    for _ in 0..300 {
        if condition() {
            return;
        }
        std::thread::sleep(Duration::from_millis(20));
    }
    panic!("timed out waiting for {description}");
}

fn wait_until_socket(directory: &std::path::Path) {
    wait_until(
        || UnixStream::connect(directory.join("sock")).is_ok(),
        "control socket",
    );
}

fn run_cli(directory: &std::path::Path, args: &[&str]) -> Output {
    Command::new(env!("CARGO_BIN_EXE_fzz"))
        .current_dir(directory)
        .args(args)
        .output()
        .expect("fzz command should run")
}

fn combined(output: &Output) -> String {
    format!(
        "{}{}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    )
}

fn scheduled_generation(output: &Output) -> u64 {
    let text = combined(output);
    text.lines()
        .find_map(|line| line.strip_prefix("scheduled generation: "))
        .unwrap_or_else(|| panic!("missing generation in {text}"))
        .parse()
        .expect("generation is numeric")
}

/// Terminal generation strictly after `after` (positive boundary for the
/// init and filesystem generations driven by the canary job).
fn await_after(directory: &std::path::Path, after: u64) -> Output {
    run_cli(
        directory,
        &[
            "control",
            "await",
            "--after",
            &after.to_string(),
            "--timeout",
            "10s",
        ],
    )
}

fn manual_silent(directory: &std::path::Path) -> bool {
    !directory.join("script-starts").exists()
}

#[test]
fn manual_target_stays_silent_at_init_and_on_matching_changes() {
    let directory = setup_directory("silent");
    let _watcher = start_watcher(&directory);
    wait_until_socket(&directory);

    // Positive boundary 1: the init generation demonstrably ran — but only
    // the canary. A manual job never runs at watcher initialization.
    wait_until(|| directory.join("canary-ran").exists(), "canary init run");
    let init_terminal = await_after(&directory, 0);
    assert!(
        combined(&init_terminal).contains("canary"),
        "init generation: {}",
        combined(&init_terminal)
    );
    assert!(
        !combined(&init_terminal).contains("await-remote"),
        "manual job must not join the init generation: {}",
        combined(&init_terminal)
    );
    assert!(
        manual_silent(&directory),
        "manual job must not execute at init"
    );

    // Positive boundary 2: filesystem events flow (root on.change matches),
    // a replacement generation runs the canary — the manual job stays
    // excluded even though root on.change would select every other job.
    std::fs::write(directory.join("src/change.txt"), "touch").unwrap();
    let init_generation = 1u64;
    let change_terminal = await_after(&directory, init_generation);
    let change_text = combined(&change_terminal);
    assert!(
        change_text.contains("canary"),
        "change generation: {change_text}"
    );
    assert!(
        !change_text.contains("await-remote"),
        "manual job must not match filesystem events: {change_text}"
    );
    assert!(
        manual_silent(&directory),
        "manual job must not execute on matching changes"
    );
}

#[test]
fn control_run_observes_blocking_script_with_exact_generation_identity() {
    let directory = setup_directory("observe");
    let _watcher = start_watcher(&directory);
    wait_until_socket(&directory);
    let socket = directory.join("sock");

    // Discovery: the recipe names the manual trigger mode.
    let list = run_cli(&directory, &["list"]);
    assert!(
        combined(&list).contains("trigger: manual"),
        "list must expose manual mode: {}",
        combined(&list)
    );

    // Alive → running under an exact, immutable identity.
    let run = run_cli(&directory, &["control", "run", "await-remote"]);
    assert!(run.status.success(), "control run: {}", combined(&run));
    let generation = scheduled_generation(&run);
    wait_until(
        || {
            let current = status(&socket);
            current["result"]["generation"].as_u64() == Some(generation)
                && current["result"]["state"].as_str() == Some("running")
        },
        "blocked script observed as running",
    );

    // Exit 0 → exactly one passed terminal result.
    std::fs::write(directory.join("release"), "pass").unwrap();
    let passed = run_cli(
        &directory,
        &[
            "control",
            "await",
            "--generation",
            &generation.to_string(),
            "--timeout",
            "10s",
        ],
    );
    let passed_text = combined(&passed);
    assert!(
        passed_text.contains("terminal reason: passed"),
        "{passed_text}"
    );
    assert!(
        passed.status.success(),
        "passed observation must exit zero: {passed_text}"
    );

    // TOON is available for agent-facing composition.
    let toon_run = run_cli(
        &directory,
        &["ctl", "--format", "toon", "run", "await-remote"],
    );
    let toon_text = combined(&toon_run);
    assert!(
        toon_run.status.success() && toon_text.contains("runId"),
        "toon run output: {toon_text}"
    );

    // Non-zero → exactly one failed terminal result with bounded evidence.
    std::fs::remove_file(directory.join("release")).unwrap();
    std::fs::write(directory.join("fail"), "fail").unwrap();
    std::fs::write(directory.join("release"), "fail").unwrap();
    let failed = run_cli(
        &directory,
        &["ctl", "run", "await-remote", "--wait", "--timeout", "10s"],
    );
    let failed_text = combined(&failed);
    assert!(
        !failed.status.success(),
        "failed observation must exit non-zero: {failed_text}"
    );
    assert!(
        failed_text.contains("terminal reason: failed"),
        "{failed_text}"
    );
    assert!(
        failed_text.contains("failure-output") && failed_text.contains("failure-error"),
        "bounded evidence must surface: {failed_text}"
    );

    // Retained output is retrievable per exact generation.
    let evidence = run_cli(
        &directory,
        &[
            "control",
            "output",
            "--generation",
            &scheduled_generation(&failed).to_string(),
        ],
    );
    assert!(
        combined(&evidence).contains("failure-output"),
        "output retrieval: {}",
        combined(&evidence)
    );
}

#[test]
fn cancelled_manual_generation_is_terminal_and_later_runs_are_isolated() {
    let directory = setup_directory("cancel");
    let _watcher = start_watcher(&directory);
    wait_until_socket(&directory);
    let socket = directory.join("sock");

    let run = run_cli(&directory, &["control", "run", "await-remote"]);
    let blocked = scheduled_generation(&run);
    wait_until(
        || {
            let current = status(&socket);
            current["result"]["generation"].as_u64() == Some(blocked)
                && current["result"]["state"].as_str() == Some("running")
        },
        "blocked generation running",
    );

    // Cancel the exact blocked generation; the process group is reaped and
    // the terminal result is distinct.
    let cancel = run_cli(
        &directory,
        &[
            "control",
            "cancel",
            "--generation",
            &blocked.to_string(),
            "--wait",
            "--timeout",
            "10s",
        ],
    );
    assert!(
        combined(&cancel).contains("terminal reason: cancelled"),
        "cancel: {}",
        combined(&cancel)
    );

    // A later explicit run is fully isolated: new identity, runs normally.
    std::fs::write(directory.join("release"), "pass").unwrap();
    let next = run_cli(&directory, &["control", "run", "await-remote"]);
    let next_generation = scheduled_generation(&next);
    assert!(
        next_generation > blocked,
        "generation identity must advance past the cancelled one"
    );
    let next_terminal = run_cli(
        &directory,
        &[
            "control",
            "await",
            "--generation",
            &next_generation.to_string(),
            "--timeout",
            "10s",
        ],
    );
    assert!(
        combined(&next_terminal).contains("terminal reason: passed"),
        "later run isolated from cancelled one: {}",
        combined(&next_terminal)
    );

    // Stale cancel against the terminal generation is a no-op: the newer
    // generation's result is unaffected by identity comparison.
    let stale = run_cli(
        &directory,
        &["control", "cancel", "--generation", &blocked.to_string()],
    );
    assert!(
        stale.status.success(),
        "stale cancel is a safe no-op: {}",
        combined(&stale)
    );
    let final_terminal = run_cli(
        &directory,
        &[
            "control",
            "await",
            "--generation",
            &next_generation.to_string(),
            "--timeout",
            "10s",
        ],
    );
    assert!(
        combined(&final_terminal).contains("terminal reason: passed"),
        "stale cancel must not affect the newer generation: {}",
        combined(&final_terminal)
    );
}

#[test]
fn local_foreground_composition_exits_with_the_script_result() {
    let directory = setup_directory("foreground");

    // A failed preceding command never starts the target (&& composition).
    let not_started = Command::new("/bin/sh")
        .arg("-c")
        .arg("false && fzz run await-remote")
        .current_dir(&directory)
        .output()
        .unwrap();
    assert!(
        !not_started.status.success(),
        "failed predecessor must fail the composition"
    );
    assert!(
        manual_silent(&directory),
        "target must not start after a failed preceding command"
    );

    // Non-zero script exit fails the composition and stops the chain.
    std::fs::write(directory.join("fail"), "fail").unwrap();
    std::fs::write(directory.join("release"), "fail").unwrap();
    let failed_chain = Command::new("/bin/sh")
        .arg("-c")
        .arg("fzz run await-remote && echo chain-continued")
        .current_dir(&directory)
        .output()
        .unwrap();
    let failed_text = format!(
        "{}{}",
        String::from_utf8_lossy(&failed_chain.stdout),
        String::from_utf8_lossy(&failed_chain.stderr)
    );
    assert!(
        !failed_chain.status.success(),
        "failed script must fail the foreground composition"
    );
    assert!(
        !failed_text.contains("chain-continued"),
        "chain must stop after a failed target: {failed_text}"
    );
    assert!(
        failed_text.contains("failure-output"),
        "failure evidence must surface locally: {failed_text}"
    );

    // Exit 0 passes the composition through to the chained command.
    std::fs::remove_file(directory.join("fail")).unwrap();
    std::fs::remove_file(directory.join("release")).unwrap();
    std::fs::write(directory.join("release"), "pass").unwrap();
    let passed_chain = Command::new("/bin/sh")
        .arg("-c")
        .arg("fzz run await-remote && echo chain-continued")
        .current_dir(&directory)
        .output()
        .unwrap();
    let passed_text = format!(
        "{}{}",
        String::from_utf8_lossy(&passed_chain.stdout),
        String::from_utf8_lossy(&passed_chain.stderr)
    );
    assert!(
        passed_chain.status.success(),
        "passing script must exit zero: {passed_text}"
    );
    assert!(
        passed_text.contains("chain-continued"),
        "chain must continue after a passing target: {passed_text}"
    );
}

fn status(socket: &std::path::Path) -> serde_json::Value {
    let mut stream = UnixStream::connect(socket).expect("connect control socket");
    std::io::Write::write_all(
        &mut stream,
        br#"{"jsonrpc":"2.0","id":"status","method":"status"}
"#,
    )
    .unwrap();
    let mut line = String::new();
    BufReader::new(stream).read_line(&mut line).unwrap();
    serde_json::from_str(&line).unwrap()
}
