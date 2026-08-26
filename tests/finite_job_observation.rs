//! TASK-0134: black-box proof for integration-agnostic finite command observation.
//!
//! A blocking script stands in for any external-system integration. The script
//! owns correlation and terminal decision; Funzzy only observes process
//! lifetime and exit status.

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

const CONFIG: &str = r#"
on:
  socket: sock
tasks:
  - name: await-remote
    run: ./await-remote.sh
    change: .funzzy-manual-never
"#;

fn setup_directory(test_name: &str, with_socket: bool) -> std::path::PathBuf {
    let counter = DIRECTORY_COUNTER.fetch_add(1, std::sync::atomic::Ordering::SeqCst);
    let directory = std::env::temp_dir().join(format!(
        "funzzy-observe-{}-{test_name}-{counter}",
        std::process::id()
    ));
    let _ = std::fs::remove_dir_all(&directory);
    std::fs::create_dir_all(&directory).unwrap();
    let config = if with_socket {
        CONFIG.to_owned()
    } else {
        CONFIG.replace("on:\n  socket: sock\n", "")
    };
    std::fs::write(directory.join(".watch.yaml"), config).unwrap();
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
    let mut permissions = std::fs::metadata(directory.join("await-remote.sh"))
        .unwrap()
        .permissions();
    permissions.set_mode(0o755);
    std::fs::set_permissions(directory.join("await-remote.sh"), permissions).unwrap();
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

#[test]
fn local_run_observes_a_blocking_script_until_exit() {
    let directory = setup_directory("local", false);
    let mut run = Command::new(env!("CARGO_BIN_EXE_fzz"))
        .current_dir(&directory)
        .args(["run", "await-remote"])
        .env("FUNZZY_COLORED", "false")
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .unwrap();

    wait_until(
        || directory.join("script-starts").exists(),
        "local script start",
    );
    assert!(
        run.try_wait().unwrap().is_none(),
        "blocked run must remain alive"
    );
    std::fs::write(directory.join("release"), "pass").unwrap();

    let output = run.wait_with_output().unwrap();
    assert!(
        output.status.success(),
        "local run failed: {}",
        combined(&output)
    );
    let text = combined(&output);
    assert!(text.contains("await-remote"), "summary: {text}");
    assert!(text.contains("passed"), "summary: {text}");
    assert!(directory.join("script-starts").exists());
}

#[test]
fn control_run_keeps_exact_blocked_generation_and_maps_exit_status() {
    let directory = setup_directory("control", true);
    let _watcher = start_watcher(&directory);
    wait_until_socket(&directory);
    let socket = directory.join("sock");

    let first = run_cli(&directory, &["control", "run", "await-remote"]);
    assert!(first.status.success(), "control run: {}", combined(&first));
    let first_id = scheduled_generation(&first);
    wait_until(
        || {
            let current = status(&socket);
            current["result"]["generation"].as_u64() == Some(first_id)
                && current["result"]["state"].as_str() == Some("running")
        },
        "first generation running",
    );
    std::fs::write(directory.join("release"), "pass").unwrap();
    let first_result = run_cli(
        &directory,
        &[
            "control",
            "await",
            "--generation",
            &first_id.to_string(),
            "--timeout",
            "10s",
        ],
    );
    assert!(
        first_result.status.success(),
        "first await: {}",
        combined(&first_result)
    );
    assert!(combined(&first_result).contains("terminal reason: passed"));

    std::fs::remove_file(directory.join("release")).unwrap();
    std::fs::write(directory.join("fail"), "fail").unwrap();
    let second = Command::new(env!("CARGO_BIN_EXE_fzz"))
        .current_dir(&directory)
        .args(["ctl", "run", "await-remote", "--wait", "--timeout", "10s"])
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .unwrap();
    wait_until(
        || {
            let current = status(&socket);
            current["result"]["state"].as_str() == Some("running")
                && current["result"]["generation"]
                    .as_u64()
                    .is_some_and(|id| id > first_id)
        },
        "second generation running",
    );
    std::fs::write(directory.join("release"), "fail").unwrap();
    let second = second.wait_with_output().unwrap();
    assert!(!second.status.success(), "failed run must exit non-zero");
    let second_id = scheduled_generation(&second);
    assert!(second_id > first_id, "run identity must advance");
    let second_text = combined(&second);
    assert!(
        second_text.contains("terminal reason: failed"),
        "{second_text}"
    );
    assert!(second_text.contains("failure-output"), "{second_text}");
    assert!(second_text.contains("failure-error"), "{second_text}");
}
