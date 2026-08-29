#![cfg(all(feature = "test-integration", unix))]

use std::io::{BufRead, BufReader, Read, Seek, SeekFrom, Write};
use std::os::unix::net::UnixStream;
use std::process::{Command, Stdio};

#[path = "./common/lib.rs"]
mod setup;

fn status(socket: &std::path::Path) -> Option<serde_json::Value> {
    let mut stream = UnixStream::connect(socket).ok()?;
    stream
        .write_all(
            br#"{"jsonrpc":"2.0","id":"status","method":"status"}
"#,
        )
        .ok()?;
    let mut line = String::new();
    BufReader::new(stream).read_line(&mut line).ok()?;
    serde_json::from_str(&line).ok()
}

#[test]
fn piped_shortcut_runs_the_full_pipeline_once() {
    setup::with_config(
        std::path::Path::new("examples/simple-case.yml"),
        "shortcut_trigger.log",
        |fzz_cmd, mut output_log, _fixture| {
            let mut child = fzz_cmd
                .stdin(Stdio::piped())
                .spawn()
                .expect("failed to spawn watcher");
            let mut input = child.stdin.take().expect("watcher stdin");
            defer!({
                let _ = child.kill();
                let _ = child.wait();
            });

            wait_until!(
                {
                    let mut output = String::new();
                    output_log.seek(SeekFrom::Start(0)).unwrap();
                    output_log.read_to_string(&mut output).unwrap();
                    output.contains("Watching...")
                },
                "watcher did not become ready"
            );

            input
                .write_all(&[funzzy::shortcut::TRIGGER_KEY])
                .expect("write shortcut");
            drop(input);

            wait_until!(
                {
                    let mut output = String::new();
                    output_log.seek(SeekFrom::Start(0)).unwrap();
                    output_log.read_to_string(&mut output).unwrap();
                    output
                        .matches("Running full pipeline from keyboard shortcut.")
                        .count()
                        == 1
                },
                "shortcut did not run all configured jobs"
            );

            let mut output = String::new();
            output_log.seek(SeekFrom::Start(0)).unwrap();
            output_log.read_to_string(&mut output).unwrap();
            assert_eq!(
                output
                    .matches("Running full pipeline from keyboard shortcut.")
                    .count(),
                1,
                "shortcut output:\n{output}"
            );
        },
    );
}

#[test]
fn socket_enabled_shortcut_reports_full_plan_status_and_output_after_busy_run() {
    setup::with_config(
        std::path::Path::new("examples/simple-case.yml"),
        "shortcut_trigger_socket.log",
        |fzz_cmd, mut output_log, fixture| {
            let socket = std::env::temp_dir().join(format!("fzz-kbd-{}.sock", std::process::id()));
            let _ = std::fs::remove_file(&socket);
            std::fs::write(
                fixture.join("examples/simple-case.yml"),
                &format!(
                    r#"on:
  change: "src/**"
  socket: "{}"
  watch_backend: poll
  poll_interval: 20ms
jobs:
  - name: first gate
    run: "sleep 3; echo first"
    change: "src/**"
    run_on_init: true
  - name: second gate
    run: "echo second"
    change: "src/**"
"#,
                    socket.display()
                ),
            )
            .expect("write socket config");
            fzz_cmd.env("FUNZZY_NON_BLOCK", "true");
            let mut child = fzz_cmd
                .stdin(Stdio::piped())
                .spawn()
                .expect("failed to spawn watcher");
            let mut input = child.stdin.take().expect("watcher stdin");
            defer!({
                let _ = child.kill();
                let _ = child.wait();
                let _ = std::fs::remove_file(&socket);
            });

            let mut startup_log = String::new();
            for _ in 0..100 {
                startup_log.clear();
                output_log.seek(SeekFrom::Start(0)).unwrap();
                output_log.read_to_string(&mut startup_log).unwrap();
                if startup_log.contains("Control socket listening") {
                    break;
                }
                std::thread::sleep(std::time::Duration::from_millis(100));
            }
            assert!(
                startup_log.contains("Control socket listening"),
                "socket watcher did not bind; output: {startup_log}"
            );
            let mut initial = None;
            for _ in 0..100 {
                if let Some(value) = status(&socket) {
                    initial = Some(value);
                    break;
                }
                std::thread::sleep(std::time::Duration::from_millis(50));
            }
            let initial = initial.unwrap_or_else(|| {
                panic!(
                    "initial status unavailable at {}; startup: {startup_log}",
                    socket.display()
                )
            });
            assert_eq!(
                initial["result"]["state"], "running",
                "initial status: {initial}"
            );
            input
                .write_all(&[funzzy::shortcut::TRIGGER_KEY])
                .expect("write shortcut press");
            wait_until!(
                {
                    let mut output = String::new();
                    output_log.seek(SeekFrom::Start(0)).unwrap();
                    output_log.read_to_string(&mut output).unwrap();
                    output.contains("Running full pipeline from keyboard shortcut.")
                },
                "latched shortcut did not start"
            );
            wait_until!(
                {
                    status(&socket).is_some_and(|value| {
                        value["result"]["generation"] == 2 && value["result"]["state"] == "running"
                    })
                },
                "shortcut generation did not become active"
            );
            input
                .write_all(&[funzzy::shortcut::TRIGGER_KEY])
                .expect("write duplicate shortcut press");
            drop(input);

            let mut generation = 0;
            wait_until!(
                {
                    match status(&socket) {
                        Some(value) => {
                            generation = value["result"]["generation"].as_u64().unwrap_or(0);
                            generation == 2 && value["result"]["state"] == "passed"
                        }
                        None => false,
                    }
                },
                "latched shortcut generation did not finish"
            );
            let current = status(&socket).expect("final status");
            assert_eq!(current["result"]["trigger"], "keyboard");
            assert_eq!(current["result"]["tasks"].as_array().map(Vec::len), Some(2));

            let output = Command::new(env!("CARGO_BIN_EXE_fzz"))
                .current_dir(fixture)
                .args([
                    "-c",
                    "examples/simple-case.yml",
                    "control",
                    "output",
                    "--generation",
                    &generation.to_string(),
                    "--full",
                ])
                .output()
                .expect("retrieve shortcut output");
            let output_text = String::from_utf8_lossy(&output.stdout);
            assert!(
                output.status.success(),
                "output retrieval failed: {output_text}"
            );
            assert!(
                output_text.contains("first"),
                "missing first output: {output_text}"
            );
            assert!(
                output_text.contains("second"),
                "missing second output: {output_text}"
            );

            let mut log = String::new();
            output_log.seek(SeekFrom::Start(0)).unwrap();
            output_log.read_to_string(&mut log).unwrap();
            assert_eq!(
                log.matches("Running full pipeline from keyboard shortcut.")
                    .count(),
                1,
                "shortcut log:\n{log}"
            );
        },
    );
}

#[test]
fn piped_shortcut_works_in_non_block_watch_mode() {
    setup::with_config(
        std::path::Path::new("examples/simple-case.yml"),
        "shortcut_trigger_non_block.log",
        |fzz_cmd, mut output_log, _fixture| {
            fzz_cmd.env("FUNZZY_NON_BLOCK", "true");
            let mut child = fzz_cmd
                .stdin(Stdio::piped())
                .spawn()
                .expect("failed to spawn watcher");
            let mut input = child.stdin.take().expect("watcher stdin");
            defer!({
                let _ = child.kill();
                let _ = child.wait();
            });

            wait_until!(
                {
                    let mut output = String::new();
                    output_log.seek(SeekFrom::Start(0)).unwrap();
                    output_log.read_to_string(&mut output).unwrap();
                    output.contains("Watching...")
                },
                "watcher did not become ready"
            );
            input
                .write_all(&[funzzy::shortcut::TRIGGER_KEY])
                .expect("write shortcut");
            drop(input);
            wait_until!(
                {
                    let mut output = String::new();
                    output_log.seek(SeekFrom::Start(0)).unwrap();
                    output_log.read_to_string(&mut output).unwrap();
                    output
                        .matches("Running full pipeline from keyboard shortcut.")
                        .count()
                        == 1
                },
                "non-block shortcut did not trigger"
            );
        },
    );
}

#[test]
fn piped_shortcut_latches_busy_run_and_ignores_extra_press() {
    setup::with_config(
        std::path::Path::new("examples/jobs-with-long-running-commands.yaml"),
        "shortcut_trigger_busy.log",
        |fzz_cmd, mut output_log, _fixture| {
            let mut child = fzz_cmd
                .stdin(Stdio::piped())
                .spawn()
                .expect("failed to spawn watcher");
            let mut input = child.stdin.take().expect("watcher stdin");
            defer!({
                let _ = child.kill();
                let _ = child.wait();
            });

            wait_until!(
                {
                    let mut output = String::new();
                    output_log.seek(SeekFrom::Start(0)).unwrap();
                    output_log.read_to_string(&mut output).unwrap();
                    output.contains("Running on init commands.")
                },
                "watcher did not start its busy generation"
            );

            input
                .write_all(&[funzzy::shortcut::TRIGGER_KEY, funzzy::shortcut::TRIGGER_KEY])
                .expect("write shortcut presses");
            drop(input);

            wait_until!(
                {
                    let mut output = String::new();
                    output_log.seek(SeekFrom::Start(0)).unwrap();
                    output_log.read_to_string(&mut output).unwrap();
                    output
                        .matches("Running full pipeline from keyboard shortcut.")
                        .count()
                        == 1
                },
                "busy shortcut did not trigger exactly once"
            );
        },
    );
}
