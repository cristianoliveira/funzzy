//! TASK-0136: black-box proof for explicit manual-only jobs
//! (docs/MANUAL-TRIGGER-CONTRACT.md).
//!
//! A manual job runs through `fzz run TARGET` and the control `run` method,
//! while filesystem changes and watcher start never trigger it.

#![cfg(all(feature = "test-integration", unix))]

use std::io::{BufRead, BufReader};
use std::os::unix::net::UnixStream;
use std::process::{Command, Stdio};
use std::time::Duration;

static DIRECTORY_COUNTER: std::sync::atomic::AtomicU32 = std::sync::atomic::AtomicU32::new(0);

const CONFIG: &str = r#"
on:
  socket: sock
  change: "src/**"
jobs:
  - name: await-remote
    trigger: manual
    run: ./await-remote.sh
  - name: build
    run: echo build-ran
    change: "src/**"
"#;

fn setup_directory(test_name: &str) -> std::path::PathBuf {
    let counter = DIRECTORY_COUNTER.fetch_add(1, std::sync::atomic::Ordering::SeqCst);
    let directory = std::env::temp_dir().join(format!(
        "funzzy-manual-{}-{test_name}-{counter}",
        std::process::id()
    ));
    let _ = std::fs::remove_dir_all(&directory);
    std::fs::create_dir_all(directory.join("src")).unwrap();
    std::fs::write(directory.join(".watch.yaml"), CONFIG).unwrap();
    std::fs::write(
        directory.join("await-remote.sh"),
        "#!/bin/sh\necho manual-ran\n",
    )
    .unwrap();
    use std::os::unix::fs::PermissionsExt;
    let mut permissions = std::fs::metadata(directory.join("await-remote.sh"))
        .unwrap()
        .permissions();
    permissions.set_mode(0o755);
    std::fs::set_permissions(directory.join("await-remote.sh"), permissions).unwrap();
    std::fs::canonicalize(&directory).unwrap()
}

fn wait_until<F: FnMut() -> bool>(mut condition: F, description: &str) {
    for _ in 0..500 {
        if condition() {
            return;
        }
        std::thread::sleep(Duration::from_millis(20));
    }
    panic!("timed out waiting for {description}");
}

fn control_run(socket: &std::path::Path, target: &str) -> serde_json::Value {
    let mut stream = UnixStream::connect(socket).expect("connect control socket");
    let request = format!(
        r#"{{"jsonrpc":"2.0","id":"run","method":"run","params":{{"target":"{target}"}}}}
"#
    );
    std::io::Write::write_all(&mut stream, request.as_bytes()).unwrap();
    let mut line = String::new();
    BufReader::new(stream).read_line(&mut line).unwrap();
    serde_json::from_str(&line).unwrap()
}

fn read_all(socket: &std::path::Path) -> String {
    let mut stream = UnixStream::connect(socket).expect("connect control socket");
    std::io::Write::write_all(
        &mut stream,
        b"{\"jsonrpc\":\"2.0\",\"id\":\"s\",\"method\":\"status\"}\n",
    )
    .unwrap();
    let mut line = String::new();
    BufReader::new(stream).read_line(&mut line).unwrap();
    line
}

fn control_status(socket: &std::path::Path) -> serde_json::Value {
    serde_json::from_str(&read_all(socket)).unwrap()
}

#[test]
fn manual_job_runs_via_fzz_run() {
    let directory = setup_directory("run");
    let output = Command::new(env!("CARGO_BIN_EXE_fzz"))
        .current_dir(&directory)
        .env("FUNZZY_COLORED", "false")
        .args(["run", "await-remote"])
        .output()
        .expect("fzz run should execute");
    let text = format!(
        "{}{}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    );
    assert!(output.status.success(), "run output: {text}");
    assert!(text.contains("manual-ran"), "manual job executed: {text}");
    let _ = std::fs::remove_dir_all(&directory);
}

#[test]
fn manual_job_never_triggers_from_filesystem_or_init_and_runs_via_control() {
    let directory = setup_directory("ctl");
    let watcher_log = std::fs::File::create(directory.join("watcher.log")).unwrap();
    let mut child = Command::new(env!("CARGO_BIN_EXE_fzz"))
        .current_dir(&directory)
        .env_remove("FUNZZY_BAIL")
        .env_remove("FUNZZY_NON_BLOCK")
        .stdout(Stdio::from(watcher_log.try_clone().unwrap()))
        .stderr(Stdio::from(watcher_log))
        .spawn()
        .unwrap();

    struct Cleanup<'a> {
        child: &'a mut std::process::Child,
        directory: std::path::PathBuf,
    }
    impl Drop for Cleanup<'_> {
        fn drop(&mut self) {
            let _ = self.child.kill();
            let _ = self.child.wait();
            let _ = std::fs::remove_dir_all(&self.directory);
        }
    }
    let _cleanup = Cleanup {
        child: &mut child,
        directory: directory.clone(),
    };

    let socket = directory.join("sock");
    wait_until(|| UnixStream::connect(&socket).is_ok(), "control socket");

    // Init ran nothing manual. Wait for the startup banner to prove the
    // manual job stayed silent at init (it would print `manual-ran`).
    let log_path = directory.join("watcher.log");
    wait_until(
        || {
            std::fs::read_to_string(&log_path)
                .map(|text| text.contains("Watching..."))
                .unwrap_or(false)
        },
        "watcher started",
    );
    // No init plan exists in this config (no run_on_init job at all), so
    // once the banner is up, initialization is complete and deterministic:
    // the manual job provably produced no output. No fixed sleeps.
    assert!(
        !std::fs::read_to_string(&log_path)
            .unwrap_or_default()
            .contains("manual-ran"),
        "manual job must never run at watcher initialization"
    );
    // Trigger the change-driven job; the manual job must stay untouched.
    std::fs::write(directory.join("src/trigger.txt"), "changed").unwrap();
    wait_until(
        || {
            std::fs::read_to_string(&log_path)
                .map(|text| text.contains("build-ran"))
                .unwrap_or(false)
        },
        "change job ran",
    );

    // Control run selects the manual job.
    let response = control_run(&socket, "await-remote");
    let generation = response["result"]["runId"].as_u64().expect("scheduled run");

    // Await the exact scheduled generation to terminal, then observe status.
    wait_until(
        || {
            let result = control_status(&socket)["result"].clone();
            let current = result["generation"].as_u64().unwrap_or(0);
            let terminal = matches!(
                result["state"].as_str(),
                Some("passed") | Some("failed") | Some("cancelled")
            );
            current >= generation && terminal
        },
        "manual generation terminal",
    );

    let status = control_status(&socket);
    assert!(
        status["result"]["trigger"]
            .as_str()
            .unwrap_or_default()
            .contains("await-remote"),
        "the terminal generation is our control run"
    );

    let log = std::fs::read_to_string(&log_path).unwrap();
    let manual_occurrences = log.matches("manual-ran").count();
    assert_eq!(
        manual_occurrences, 1,
        "manual-ran exactly once (control run), never from init or file change"
    );
}

/// MANUAL-TRIGGER-CONTRACT §3.5 matrix (Kely-verified manually; automated
/// here): a watch selection containing ONLY manual jobs is a usage error
/// without a control socket, and a valid control-only watcher with one.
#[test]
fn all_manual_watch_matrix_socketless_error_vs_control_only_valid() {
    // Socketless: immediate usage error, nonzero exit, no watcher started.
    let directory = setup_directory("matrix-socketless");
    std::fs::write(
        directory.join(".watch.yaml"),
        "jobs:\n  - name: await-remote\n    trigger: manual\n    run: ./await-remote.sh\n",
    )
    .unwrap();
    let output = Command::new(env!("CARGO_BIN_EXE_fzz"))
        .current_dir(&directory)
        .env("FUNZZY_COLORED", "false")
        .arg("watch")
        .output()
        .expect("fzz watch should run");
    let text = format!(
        "{}{}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    );
    assert!(!output.status.success(), "socketless all-manual must fail");
    assert!(
        text.contains("Nothing to watch"),
        "usage error is actionable: {text}"
    );
    let _ = std::fs::remove_dir_all(&directory);

    // With on.socket: a valid control-only watcher stays up serving ctl run.
    let directory = setup_directory("matrix-control-only");
    std::fs::write(
        directory.join(".watch.yaml"),
        "on:\n  socket: sock\njobs:\n  - name: await-remote\n    trigger: manual\n    run: ./await-remote.sh\n",
    )
    .unwrap();
    let mut child = Command::new(env!("CARGO_BIN_EXE_fzz"))
        .current_dir(&directory)
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .arg("watch")
        .spawn()
        .expect("control-only watcher starts");

    let socket = directory.join("sock");
    wait_until(
        || UnixStream::connect(&socket).is_ok(),
        "control-only watcher socket",
    );

    // The watcher is serving: an explicit control run of the manual job
    // schedules and reaches terminal — proof it is a live control surface,
    // not a dead watcher.
    let response = control_run(&socket, "await-remote");
    let generation = response["result"]["runId"].as_u64().expect("scheduled run");
    wait_until(
        || {
            let result = control_status(&socket)["result"].clone();
            result["generation"].as_u64().unwrap_or(0) >= generation
                && matches!(
                    result["state"].as_str(),
                    Some("passed") | Some("failed") | Some("cancelled")
                )
        },
        "manual generation terminal on control-only watcher",
    );

    let _ = child.kill();
    let _ = child.wait();
    let _ = std::fs::remove_dir_all(&directory);
}
