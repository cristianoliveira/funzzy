//! TASK-0167: black-box proof for the explicit root watch-target shorthand.

#![cfg(all(feature = "test-integration", unix))]

use std::cell::RefCell;
use std::io::{BufRead, BufReader, Write};
use std::os::unix::net::UnixStream;
use std::path::{Path, PathBuf};
use std::process::{Child, Command};
use std::sync::atomic::{AtomicU32, Ordering};
use std::time::Duration;

#[path = "./common/lib.rs"]
mod setup;

static SOCKET_COUNTER: AtomicU32 = AtomicU32::new(0);

fn socket_path() -> PathBuf {
    let counter = SOCKET_COUNTER.fetch_add(1, Ordering::SeqCst);
    std::env::temp_dir().join(format!("fzz-target-{}-{counter}", std::process::id()))
}

fn status(socket: &Path) -> Option<serde_json::Value> {
    let mut stream = UnixStream::connect(socket).ok()?;
    writeln!(
        stream,
        r#"{{"jsonrpc":"2.0","id":"status","method":"status"}}"#
    )
    .ok()?;
    let mut line = String::new();
    BufReader::new(stream).read_line(&mut line).ok()?;
    serde_json::from_str(&line).ok()
}

fn wait_for_passing_status(socket: &Path, expected_tasks: usize) -> serde_json::Value {
    for _ in 0..100 {
        if let Some(response) = status(socket) {
            let result = &response["result"];
            if result["state"] == "passed"
                || result["tasks"].as_array().is_some_and(|tasks| {
                    tasks.len() == expected_tasks
                        && tasks.iter().all(|task| task["state"] == "passed")
                })
            {
                return result.clone();
            }
        }
        std::thread::sleep(Duration::from_millis(250));
    }
    panic!(
        "watcher should expose a passing initial generation: {:?}",
        status(socket)
    );
}

fn task_names(result: &serde_json::Value) -> Vec<String> {
    result["tasks"]
        .as_array()
        .into_iter()
        .flatten()
        .filter_map(|task| task["name"].as_str().map(str::to_owned))
        .collect()
}

fn watcher_command(fixture: &Path) -> Command {
    let mut command = Command::new(env!("CARGO_BIN_EXE_fzz"));
    command
        .current_dir(fixture)
        .env_remove("FUNZZY_BAIL")
        .env_remove("FUNZZY_NON_BLOCK")
        .env("FUNZZY_COLORED", "false");
    command
}

fn config_with_socket(config: &str, socket: &Path) -> String {
    config.replace(
        "socket: .watch.sock",
        &format!("socket: '{}'", socket.display()),
    )
}

fn stop_watcher(child: &mut Child) {
    let pid = child.id().to_string();
    let _ = std::process::Command::new("kill")
        .args(["-INT", &pid])
        .status();
    for _ in 0..150 {
        if child.try_wait().ok().flatten().is_some() {
            return;
        }
        std::thread::sleep(Duration::from_millis(20));
    }
    let _ = child.kill();
    let _ = child.wait();
}

const TARGET_CONFIG: &str = r#"
on:
  socket: .watch.sock
jobs:
  - name: first @quick
    run: "echo first >> selected.log"
    change: "src/**"
    run_on_init: true
  - name: second @quick
    run: "echo second >> selected.log"
    change: "src/**"
    run_on_init: true
  - name: skipped @slow
    run: "echo skipped >> skipped.log"
    change: "src/**"
    run_on_init: true
"#;

const MULTI_MATCH_CONFIG: &str = r#"
on:
  socket: .watch.sock
jobs:
  - name: build frontend
    run: "echo frontend >> selected.log"
    change: "src/**"
    run_on_init: true
  - name: build backend
    run: "echo backend >> selected.log"
    change: "src/**"
    run_on_init: true
"#;

#[test]
fn shorthand_has_the_same_effective_plan_as_watch_subcommand() {
    setup::with_output("target-shorthand-plan.log", |_, _, fixture| {
        let socket = socket_path();
        std::fs::write(
            fixture.join(".watch.yaml"),
            config_with_socket(TARGET_CONFIG, &socket),
        )
        .unwrap();

        let child = RefCell::new(Some(
            watcher_command(fixture)
                .args(["watch", "@quick"])
                .spawn()
                .expect("explicit watch should start"),
        ));
        defer!({
            if let Some(mut child) = child.borrow_mut().take() {
                stop_watcher(&mut child);
            }
            let _ = std::fs::remove_file(&socket);
        });
        let explicit_status = wait_for_passing_status(&socket, 2);
        assert_eq!(
            task_names(&explicit_status),
            vec!["first @quick", "second @quick"]
        );
        assert!(!fixture.join("skipped.log").exists());
        let mut explicit = child.borrow_mut().take().expect("explicit watcher exists");
        stop_watcher(&mut explicit);
        let _ = std::fs::remove_file(&socket);
        *child.borrow_mut() = Some(
            watcher_command(fixture)
                .args(["--", "@quick"])
                .spawn()
                .expect("shorthand watch should start"),
        );
        let shorthand_status = wait_for_passing_status(&socket, 2);
        assert_eq!(task_names(&shorthand_status), task_names(&explicit_status));
        assert_eq!(
            shorthand_status["services"], explicit_status["services"],
            "shorthand and explicit watch should expose the same services"
        );
        assert!(!fixture.join("skipped.log").exists());
    });
}

#[test]
fn shorthand_preserves_existing_multi_match_watch_selection() {
    setup::with_output("target-shorthand-multi.log", |_, _, fixture| {
        let socket = socket_path();
        std::fs::write(
            fixture.join(".watch.yaml"),
            config_with_socket(MULTI_MATCH_CONFIG, &socket),
        )
        .unwrap();
        let mut child = watcher_command(fixture)
            .args(["--", "build"])
            .spawn()
            .expect("multi-match shorthand should start");
        defer!({
            stop_watcher(&mut child);
            let _ = std::fs::remove_file(&socket);
        });

        let result = wait_for_passing_status(&socket, 2);
        assert_eq!(task_names(&result), vec!["build frontend", "build backend"]);
    });
}

#[test]
fn missing_shorthand_target_keeps_watch_diagnostics() {
    setup::with_output("target-shorthand-missing.log", |_, _, fixture| {
        let socket = socket_path();
        let config = r#"
on:
  socket: .watch.sock
jobs:
  - name: build frontend
    run: "true"
    change: "src/**"
  - name: build backend
    run: "true"
    change: "src/**"
"#;
        std::fs::write(
            fixture.join(".watch.yaml"),
            config_with_socket(config, &socket),
        )
        .unwrap();

        let output = watcher_command(fixture)
            .args(["--", "unknown"])
            .output()
            .expect("invalid shorthand should return a usage result");
        assert_eq!(output.status.code(), Some(1));
        let stdout = String::from_utf8_lossy(&output.stdout);
        assert!(
            stdout.contains("No target found for 'unknown'"),
            "unexpected output: {stdout}"
        );
        assert!(!socket.exists(), "invalid target must not start a socket");
        let _ = std::fs::remove_file(&socket);
    });
}

#[test]
fn extra_shorthand_values_fail_before_configuration_load() {
    setup::with_output("target-shorthand-extra.log", |_, _, fixture| {
        let output = watcher_command(fixture)
            .args(["-c", "missing.yaml", "--", "@quick", "@slow"])
            .output()
            .expect("extra shorthand values should return a parse result");
        assert_eq!(output.status.code(), Some(2));
        let stderr = String::from_utf8_lossy(&output.stderr);
        assert!(stderr.contains("no more were expected"), "stderr: {stderr}");
    });
}

#[test]
fn shorthand_composes_no_services_before_target_selection() {
    setup::with_output("target-shorthand-no-services.log", |_, _, fixture| {
        let config = r#"
on:
  socket: .watch.sock
jobs:
  - name: finite @quick
    run: "echo finite > finite.done"
    change: "src/**"
    run_on_init: true
  - name: service @quick
    service: true
    run: "echo service > service.started; while :; do sleep 1; done"
    change: "src/**"
    run_on_init: true
"#;
        let socket = socket_path();
        std::fs::write(
            fixture.join(".watch.yaml"),
            config_with_socket(config, &socket),
        )
        .unwrap();

        let mut child = watcher_command(fixture)
            .args(["--no-services", "--", "@quick"])
            .spawn()
            .expect("composed shorthand should start");
        defer!({
            stop_watcher(&mut child);
            let _ = std::fs::remove_file(&socket);
        });

        let result = wait_for_passing_status(&socket, 1);
        assert_eq!(task_names(&result), vec!["finite @quick"]);
        assert!(fixture.join("finite.done").exists());
        assert!(!fixture.join("service.started").exists());
        assert!(result["services"]
            .as_array()
            .is_none_or(|services| services.is_empty()));
    });
}
