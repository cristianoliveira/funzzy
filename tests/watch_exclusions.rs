//! TASK-0165: black-box proof for invocation-only watch exclusions.
//!
//! These tests start the real watcher and inspect its real control socket and
//! process-visible marker files. Excluded jobs must never reach process
//! ownership, while the remaining finite plan still runs to a terminal state.

#![cfg(all(feature = "test-integration", unix))]

use std::io::{BufRead, BufReader, Write};
use std::os::unix::net::UnixStream;
use std::process::{Child, Command, Output, Stdio};
use std::sync::atomic::{AtomicU32, Ordering};
use std::time::{Duration, Instant};

static DIRECTORY_COUNTER: AtomicU32 = AtomicU32::new(0);

struct Watcher {
    child: Child,
    directory: std::path::PathBuf,
}

impl Drop for Watcher {
    fn drop(&mut self) {
        let pid = self.child.id().to_string();
        let _ = Command::new("kill").args(["-INT", &pid]).status();
        let deadline = Instant::now() + Duration::from_secs(3);
        loop {
            match self.child.try_wait() {
                Ok(Some(_)) => break,
                Ok(None) if Instant::now() < deadline => {
                    std::thread::sleep(Duration::from_millis(20));
                }
                _ => {
                    let _ = self.child.kill();
                    let _ = self.child.wait();
                    break;
                }
            }
        }
        let _ = std::fs::remove_dir_all(&self.directory);
    }
}

fn setup_directory(config: &str) -> std::path::PathBuf {
    let counter = DIRECTORY_COUNTER.fetch_add(1, Ordering::SeqCst);
    // Keep the socket path below macOS' `sun_path` limit. The process ID and
    // counter already provide uniqueness.
    let directory = std::env::temp_dir().join(format!("fzzx-{}-{counter}", std::process::id()));

    let _ = std::fs::remove_dir_all(&directory);
    std::fs::create_dir_all(directory.join("src")).unwrap();
    std::fs::write(directory.join(".watch.yaml"), config).unwrap();
    std::fs::canonicalize(directory).unwrap()
}

fn start_watcher(directory: &std::path::Path, args: &[&str]) -> Watcher {
    let log = std::fs::File::create(directory.join("watcher.log")).unwrap();
    let child = Command::new(env!("CARGO_BIN_EXE_fzz"))
        .current_dir(directory)
        .args(args)
        .env_remove("FUNZZY_BAIL")
        .env_remove("FUNZZY_NON_BLOCK")
        .env("FUNZZY_COLORED", "false")
        .stdout(Stdio::from(log.try_clone().unwrap()))
        .stderr(Stdio::from(log))
        .spawn()
        .unwrap();
    Watcher {
        child,
        directory: directory.to_path_buf(),
    }
}

fn run_cli(directory: &std::path::Path, args: &[&str]) -> Output {
    Command::new(env!("CARGO_BIN_EXE_fzz"))
        .current_dir(directory)
        .args(args)
        .env("FUNZZY_COLORED", "false")
        .output()
        .expect("fzz command should run")
}

fn wait_until<F: FnMut() -> bool>(mut condition: F, description: &str) {
    let deadline = Instant::now() + Duration::from_secs(20);
    loop {
        if condition() {
            return;
        }
        assert!(
            Instant::now() < deadline,
            "timed out waiting for {description}"
        );
        std::thread::sleep(Duration::from_millis(20));
    }
}

fn status(socket: &std::path::Path) -> serde_json::Value {
    let mut stream = UnixStream::connect(socket).expect("connect control socket");
    writeln!(
        stream,
        r#"{{"jsonrpc":"2.0","id":"status","method":"status"}}"#
    )
    .unwrap();
    let mut line = String::new();
    BufReader::new(stream).read_line(&mut line).unwrap();
    serde_json::from_str(&line).unwrap()
}

fn wait_until_status<F: FnMut(&serde_json::Value) -> bool>(
    socket: &std::path::Path,
    mut condition: F,
    description: &str,
) -> serde_json::Value {
    let mut last = serde_json::Value::Null;
    wait_until(
        || {
            last = match status(socket).get("result") {
                Some(result) => result.clone(),
                None => serde_json::Value::Null,
            };
            condition(&last)
        },
        description,
    );
    last
}

fn names(result: &serde_json::Value) -> Vec<String> {
    result["tasks"]
        .as_array()
        .into_iter()
        .flatten()
        .filter_map(|task| task["name"].as_str().map(str::to_owned))
        .collect()
}

const SERVICE_CONFIG: &str = r#"
on:
  socket: sock
jobs:
  - name: finite
    run: "echo finite > finite.done"
    change: "src/**"
    run_on_init: true
  - name: legacy-service
    service: true
    run: "echo legacy > legacy.started; while :; do sleep 1; done"
    change: "src/**"
    run_on_init: true
  - name: ready-service
    service: true
    run: "echo ready > ready.started; while :; do sleep 1; done"
    change: "src/**"
    run_on_init: true
    readiness:
      run: "test -f ready.ok"
      timeout: 5s
      interval: 20ms
"#;

#[test]
fn no_services_runs_finite_job_without_starting_either_service_kind() {
    let directory = setup_directory(SERVICE_CONFIG);
    let _watcher = start_watcher(&directory, &["watch", "--no-services"]);
    let socket = directory.join("sock");

    wait_until(|| socket.exists(), "control socket");
    wait_until(|| directory.join("finite.done").exists(), "finite init job");
    let result = wait_until_status(
        &socket,
        |result| result["state"] == "passed" && result["generation"].as_u64().unwrap_or(0) > 0,
        "finite generation to pass",
    );

    assert_eq!(names(&result), vec!["finite"]);
    assert!(result["services"]
        .as_array()
        .is_none_or(|services| services.is_empty()));
    assert!(!directory.join("legacy.started").exists());
    assert!(!directory.join("ready.started").exists());
}

#[test]
fn tag_exclusion_keeps_other_finite_jobs_in_declaration_order() {
    let directory = setup_directory(
        r#"
on:
  socket: sock
jobs:
  - name: first @quick
    run: 'echo first >> order.log'
    change: "src/**"
    run_on_init: true
  - name: skipped-one @slow
    run: 'echo skipped-one >> skipped-one.started'
    change: "src/**"
    run_on_init: true
  - name: skipped-two @slow
    run: 'echo skipped-two >> skipped-two.started'
    change: "src/**"
    run_on_init: true
  - name: last
    run: 'echo last >> order.log'
    change: "src/**"
    run_on_init: true
"#,
    );
    let _watcher = start_watcher(&directory, &["watch", "--exclude", "@slow"]);
    let socket = directory.join("sock");

    wait_until(|| socket.exists(), "control socket");
    wait_until(
        || directory.join("order.log").exists(),
        "remaining finite jobs",
    );
    let result = wait_until_status(
        &socket,
        |result| result["state"] == "passed",
        "filtered generation to pass",
    );

    assert_eq!(names(&result), vec!["first @quick", "last"]);
    assert_eq!(
        std::fs::read_to_string(directory.join("order.log")).unwrap(),
        "first\nlast\n"
    );
    assert!(!directory.join("skipped-one.started").exists());
    assert!(!directory.join("skipped-two.started").exists());
}

#[test]
fn positive_target_repeated_exclusion_and_no_services_compose_without_widening() {
    let directory = setup_directory(
        r#"
on:
  socket: sock
jobs:
  - name: build @quick
    run: 'echo build > build.done'
    change: "src/**"
    run_on_init: true
  - name: lint @quick
    run: 'echo lint > lint.done'
    change: "src/**"
    run_on_init: true
  - name: server @quick
    service: true
    run: 'echo server > server.started; while :; do sleep 1; done'
    change: "src/**"
    run_on_init: true
"#,
    );
    let _watcher = start_watcher(
        &directory,
        &[
            "watch",
            "@quick",
            "--exclude",
            "build",
            "--exclude",
            "build",
            "--no-services",
        ],
    );
    let socket = directory.join("sock");

    wait_until(|| socket.exists(), "control socket");
    wait_until(
        || directory.join("lint.done").exists(),
        "selected finite job",
    );
    let result = wait_until_status(
        &socket,
        |result| result["state"] == "passed",
        "composed filtered generation to pass",
    );

    assert_eq!(names(&result), vec!["lint @quick"]);
    assert!(!directory.join("build.done").exists());
    assert!(!directory.join("server.started").exists());
    assert!(result["services"]
        .as_array()
        .is_none_or(|services| services.is_empty()));
}

#[test]
fn invalid_exclusions_exit_with_actionable_usage_errors_before_startup() {
    let directory = setup_directory(
        r#"
jobs:
  - name: build
    run: 'echo build > build.started'
    change: "src/**"
  - name: lint
    run: 'echo lint > lint.started'
    change: "src/**"
  - name: lint docs
    run: 'echo docs > docs.started'
    change: "src/**"
"#,
    );

    for (args, message) in [
        (
            vec!["watch", "--exclude", "lin"],
            "is ambiguous; matches: lint, lint docs",
        ),
        (
            vec!["watch", "--exclude", "missing"],
            "No target found for exclusion 'missing'",
        ),
        (
            vec!["watch", "build", "--exclude", "build"],
            "no runnable jobs",
        ),
    ] {
        let output = run_cli(&directory, &args);
        let combined = format!(
            "{}{}",
            String::from_utf8_lossy(&output.stdout),
            String::from_utf8_lossy(&output.stderr)
        );
        assert_eq!(output.status.code(), Some(2), "output: {combined}");
        assert!(
            combined.contains(message),
            "missing {message:?}: {combined}"
        );
        assert!(!directory.join("sock").exists());
    }
    assert!(!directory.join("build.started").exists());
    assert!(!directory.join("lint.started").exists());
    assert!(!directory.join("docs.started").exists());
}
