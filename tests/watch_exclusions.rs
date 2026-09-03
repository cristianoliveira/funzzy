//! TASK-0165: black-box proof for invocation-only watch exclusions.
//!
//! These tests start the real watcher and inspect its real control socket and
//! process-visible marker files. Excluded jobs must never reach process
//! ownership, while the remaining finite plan still runs to a terminal state.

#![cfg(all(feature = "test-integration", unix))]

use std::io::{BufRead, BufReader, Write};
use std::os::unix::net::UnixStream;
use std::path::{Path, PathBuf};
use std::process::{Child, Command, Output};
use std::sync::atomic::{AtomicU32, Ordering};

#[path = "./common/lib.rs"]
mod setup;

static SOCKET_COUNTER: AtomicU32 = AtomicU32::new(0);

fn socket_path() -> PathBuf {
    let counter = SOCKET_COUNTER.fetch_add(1, Ordering::SeqCst);
    // Keep the socket path below macOS' `sun_path` limit. The common fixture
    // helper intentionally uses descriptive paths, so the socket itself gets
    // a short unique absolute path.
    std::env::temp_dir().join(format!("fzzx-sock-{}-{counter}", std::process::id()))
}

fn write_config(fixture: &Path, config: &str, socket: Option<&Path>) {
    let config = socket
        .map(|socket| config.replace("socket: sock", &format!("socket: '{}'", socket.display())))
        .unwrap_or_else(|| config.to_owned());
    std::fs::write(fixture.join(".watch.yaml"), config).unwrap();
}

fn stop_watcher(child: &mut Child) {
    let pid = child.id().to_string();
    let _ = Command::new("kill").args(["-INT", &pid]).status();
    for _ in 0..150 {
        if child.try_wait().ok().flatten().is_some() {
            return;
        }
        std::thread::sleep(std::time::Duration::from_millis(20));
    }
    let _ = child.kill();
    let _ = child.wait();
}

fn run_cli(directory: &Path, args: &[&str]) -> Output {
    Command::new(env!("CARGO_BIN_EXE_fzz"))
        .current_dir(directory)
        .args(args)
        .env_remove("FUNZZY_BAIL")
        .env_remove("FUNZZY_NON_BLOCK")
        .env("FUNZZY_COLORED", "false")
        .output()
        .expect("fzz command should run")
}

fn try_status(socket: &Path) -> Option<serde_json::Value> {
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

fn result_after_terminal(socket: &Path) -> serde_json::Value {
    wait_until!(
        { try_status(socket).is_some_and(|response| response["result"]["state"] == "passed") },
        "filtered generation to pass"
    );
    try_status(socket)
        .and_then(|response| response.get("result").cloned())
        .expect("terminal status should remain available")
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
    setup::with_output("watch-exclusions-no-services.log", |fzz_cmd, _, fixture| {
        let socket = socket_path();
        write_config(fixture, SERVICE_CONFIG, Some(&socket));
        let mut child = fzz_cmd
            .args(["watch", "--no-services"])
            .spawn()
            .expect("watcher should start");
        defer!({
            stop_watcher(&mut child);
            let _ = std::fs::remove_file(&socket);
        });

        wait_until!(
            { try_status(&socket).is_some() },
            "control socket should be connectable"
        );
        wait_until!(
            { fixture.join("finite.done").exists() },
            "finite init job should run"
        );
        let result = result_after_terminal(&socket);

        assert_eq!(names(&result), vec!["finite"]);
        assert!(result["services"]
            .as_array()
            .is_none_or(|services| services.is_empty()));
        assert!(!fixture.join("legacy.started").exists());
        assert!(!fixture.join("ready.started").exists());
    });
}

#[test]
fn tag_exclusion_keeps_other_finite_jobs_in_declaration_order() {
    setup::with_output("watch-exclusions-tag-order.log", |fzz_cmd, _, fixture| {
        let socket = socket_path();
        let config = r#"
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
"#;
        write_config(fixture, config, Some(&socket));
        let mut child = fzz_cmd
            .args(["watch", "--exclude", "@slow"])
            .spawn()
            .expect("watcher should start");
        defer!({
            stop_watcher(&mut child);
            let _ = std::fs::remove_file(&socket);
        });

        wait_until!(
            { try_status(&socket).is_some() },
            "control socket should be connectable"
        );
        wait_until!(
            { fixture.join("order.log").exists() },
            "remaining finite jobs should run"
        );
        let result = result_after_terminal(&socket);

        assert_eq!(names(&result), vec!["first @quick", "last"]);
        assert_eq!(
            std::fs::read_to_string(fixture.join("order.log")).unwrap(),
            "first\nlast\n"
        );
        assert!(!fixture.join("skipped-one.started").exists());
        assert!(!fixture.join("skipped-two.started").exists());
    });
}

#[test]
fn positive_target_repeated_exclusion_and_no_services_compose_without_widening() {
    setup::with_output("watch-exclusions-composition.log", |fzz_cmd, _, fixture| {
        let socket = socket_path();
        let config = r#"
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
"#;
        write_config(fixture, config, Some(&socket));
        let mut child = fzz_cmd
            .args([
                "watch",
                "@quick",
                "--exclude",
                "build",
                "--exclude",
                "build",
                "--no-services",
            ])
            .spawn()
            .expect("watcher should start");
        defer!({
            stop_watcher(&mut child);
            let _ = std::fs::remove_file(&socket);
        });

        wait_until!(
            { try_status(&socket).is_some() },
            "control socket should be connectable"
        );
        wait_until!(
            { fixture.join("lint.done").exists() },
            "selected finite job should run"
        );
        let result = result_after_terminal(&socket);

        assert_eq!(names(&result), vec!["lint @quick"]);
        assert!(!fixture.join("build.done").exists());
        assert!(!fixture.join("server.started").exists());
        assert!(result["services"]
            .as_array()
            .is_none_or(|services| services.is_empty()));
    });
}

#[test]
fn invalid_exclusions_exit_with_actionable_usage_errors_before_startup() {
    setup::with_output("watch-exclusions-invalid.log", |_, _, fixture| {
        let config = r#"
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
"#;
        write_config(fixture, config, None);

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
            let output = run_cli(fixture, &args);
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
            assert!(!fixture.join("sock").exists());
        }
        assert!(!fixture.join("build.started").exists());
        assert!(!fixture.join("lint.started").exists());
        assert!(!fixture.join("docs.started").exists());
    });
}
