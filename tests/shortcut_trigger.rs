#![cfg(all(feature = "test-integration", unix))]

use std::fs::File;
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

fn retry_keys_config(socket: Option<&std::path::Path>) -> String {
    let socket_line = socket
        .map(|path| format!("  socket: \"{}\"\n", path.display()))
        .unwrap_or_default();
    format!(
        r#"on:
{socket_line}  watch_backend: poll
  poll_interval: 20ms
jobs:
  - name: fails
    run: 'echo fail >> fails.log; if test -f allow; then exit 0; else exit 1; fi'
    change: 'src/**'
  - name: passes
    run: 'echo pass >> passes.log'
    change: 'src/**'
"#
    )
}

fn write_retry_config(fixture: &std::path::Path, socket: Option<&std::path::Path>) {
    std::fs::write(
        fixture.join("examples/retry-keys.yaml"),
        retry_keys_config(socket),
    )
    .expect("write retry config");
}

fn read_fixture_log(fixture: &std::path::Path, name: &str) -> usize {
    std::fs::read_to_string(fixture.join(name))
        .map(|content| content.lines().count())
        .unwrap_or(0)
}

fn watcher_output(output_log: &mut File) -> String {
    let mut output = String::new();
    output_log.seek(SeekFrom::Start(0)).unwrap();
    output_log.read_to_string(&mut output).unwrap();
    output
}

fn trigger_change(fixture: &std::path::Path) {
    std::fs::write(fixture.join("src/trigger.txt"), "retry me").expect("write trigger");
}

fn press(input: &mut std::process::ChildStdin, key: u8) {
    input.write_all(&[key]).expect("write shortcut key");
}

const CTRL_R: u8 = 0x12;

#[test]
fn piped_ctrl_r_retries_only_failed_blocking_jobs() {
    setup::with_config(
        std::path::Path::new("examples/retry-keys.yaml"),
        "shortcut_ctrl_r_blocking.log",
        |fzz_cmd, mut output_log, fixture| {
            write_retry_config(fixture, None);
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
                watcher_output(&mut output_log).contains("Watching..."),
                "watcher did not become ready"
            );
            trigger_change(fixture);
            wait_until!(
                read_fixture_log(fixture, "fails.log") == 1
                    && read_fixture_log(fixture, "passes.log") == 1,
                "source run did not complete both jobs"
            );

            std::fs::write(fixture.join("allow"), "passes now").expect("allow retry");
            press(&mut input, CTRL_R);
            wait_until!(
                watcher_output(&mut output_log)
                    .contains("Retrying failed jobs from keyboard shortcut."),
                "keyboard retry did not run"
            );
            wait_until!(
                read_fixture_log(fixture, "fails.log") == 2,
                "failed job was not retried"
            );

            let output = watcher_output(&mut output_log);
            assert_eq!(
                output
                    .matches("Retrying failed jobs from keyboard shortcut.")
                    .count(),
                1,
                "retry output:\n{output}"
            );
            assert_eq!(
                read_fixture_log(fixture, "passes.log"),
                1,
                "passed job must never be retried"
            );
            assert!(
                !output.contains("Running full pipeline from keyboard shortcut."),
                "retry must never fall back to a full run:\n{output}"
            );
        },
    );
}

#[test]
fn piped_ctrl_r_without_a_previous_run_is_rejected_without_a_full_run() {
    setup::with_config(
        std::path::Path::new("examples/retry-keys.yaml"),
        "shortcut_ctrl_r_no_run.log",
        |fzz_cmd, mut output_log, fixture| {
            write_retry_config(fixture, None);
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
                watcher_output(&mut output_log).contains("Watching..."),
                "watcher did not become ready"
            );
            press(&mut input, CTRL_R);
            wait_until!(
                watcher_output(&mut output_log)
                    .contains("Shortcut ignored: no previous run to retry."),
                "keyboard retry without a previous run was not rejected"
            );

            assert_eq!(
                read_fixture_log(fixture, "fails.log"),
                0,
                "rejection must not schedule any work"
            );
            assert!(
                !watcher_output(&mut output_log)
                    .contains("Running full pipeline from keyboard shortcut."),
                "rejection must not fall back to a full run"
            );
        },
    );
}

#[test]
fn socket_watch_ctrl_r_retries_the_latest_terminal_generation() {
    let socket = std::env::temp_dir().join(format!("fzz-kbd-retry-{}.sock", std::process::id()));
    let _ = std::fs::remove_file(&socket);
    setup::with_config(
        std::path::Path::new("examples/retry-keys.yaml"),
        "shortcut_ctrl_r_socket.log",
        move |fzz_cmd, mut output_log, fixture| {
            write_retry_config(fixture, Some(&socket));
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

            wait_until!(
                watcher_output(&mut output_log).contains("Control socket listening"),
                "socket watcher did not bind"
            );
            trigger_change(fixture);
            wait_until!(
                status(std::path::Path::new(&socket))
                    .is_some_and(|value| { value["result"]["state"] == "failed" }),
                "source generation did not fail"
            );
            let source = status(std::path::Path::new(&socket)).unwrap()["result"]["generation"]
                .as_u64()
                .unwrap();

            std::fs::write(fixture.join("allow"), "passes now").expect("allow retry");
            press(&mut input, CTRL_R);
            wait_until!(
                watcher_output(&mut output_log)
                    .contains("Retrying failed jobs from keyboard shortcut."),
                "keyboard retry did not schedule"
            );
            wait_until!(
                status(std::path::Path::new(&socket)).is_some_and(|value| {
                    value["result"]["generation"].as_u64() == Some(source + 1)
                        && value["result"]["state"] == "passed"
                        && value["result"]["trigger"] == format!("retry:{source}")
                }),
                "retry generation did not pass"
            );

            wait_until!(
                read_fixture_log(fixture, "fails.log") == 2,
                "failed job was not retried"
            );
            assert_eq!(
                read_fixture_log(fixture, "passes.log"),
                1,
                "passed job must never be retried"
            );
        },
    );
}

#[test]
fn piped_ctrl_r_burst_presses_during_a_blocking_run_retries_once() {
    let config = r#"on:
  watch_backend: poll
  poll_interval: 20ms
jobs:
  - name: slow-fails
    run: 'echo start >> slow.log; sleep 3; if test -f allow; then exit 0; else exit 1; fi'
    change: 'src/**'
"#;
    setup::with_config(
        std::path::Path::new("examples/retry-keys.yaml"),
        "shortcut_ctrl_r_latch.log",
        |fzz_cmd, mut output_log, fixture| {
            std::fs::write(fixture.join("examples/retry-keys.yaml"), config)
                .expect("write burst config");
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
                watcher_output(&mut output_log).contains("Watching..."),
                "watcher did not become ready"
            );
            trigger_change(fixture);
            wait_until!(
                read_fixture_log(fixture, "slow.log") == 1,
                "active run did not start"
            );

            // Both presses queue while the blocking run holds the loop; the
            // first action wins and the duplicate is dropped.
            press(&mut input, CTRL_R);
            press(&mut input, CTRL_R);
            wait_until!(
                read_fixture_log(fixture, "slow.log") == 2,
                "the first shortcut action did not retry after the terminal run"
            );

            let output = watcher_output(&mut output_log);
            assert_eq!(
                output
                    .matches("Retrying failed jobs from keyboard shortcut.")
                    .count(),
                1,
                "duplicate presses must never retry twice:\n{output}"
            );
            assert!(
                !output.contains("Running full pipeline from keyboard shortcut."),
                "retry must never fall back to a full run:\n{output}"
            );
        },
    );
}

#[test]
fn piped_ctrl_r_ctrl_g_burst_during_a_blocking_run_runs_only_the_first_action() {
    let config = r#"on:
  watch_backend: poll
  poll_interval: 20ms
jobs:
  - name: slow-fails
    run: 'echo start >> slow.log; sleep 3; if test -f allow; then exit 0; else exit 1; fi'
    change: 'src/**'
"#;
    setup::with_config(
        std::path::Path::new("examples/retry-keys.yaml"),
        "shortcut_ctrl_mixed_burst.log",
        |fzz_cmd, mut output_log, fixture| {
            std::fs::write(fixture.join("examples/retry-keys.yaml"), config)
                .expect("write mixed burst config");
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
                watcher_output(&mut output_log).contains("Watching..."),
                "watcher did not become ready"
            );
            trigger_change(fixture);
            wait_until!(
                read_fixture_log(fixture, "slow.log") == 1,
                "active run did not start"
            );

            // Ctrl-R queued first wins; the later Ctrl-G in the same burst is
            // dropped instead of escalating the retry into a full run.
            press(&mut input, CTRL_R);
            press(&mut input, funzzy::shortcut::TRIGGER_KEY);
            wait_until!(
                read_fixture_log(fixture, "slow.log") == 2,
                "the first shortcut action did not retry after the terminal run"
            );

            let output = watcher_output(&mut output_log);
            assert_eq!(
                output
                    .matches("Retrying failed jobs from keyboard shortcut.")
                    .count(),
                1,
                "burst output:\n{output}"
            );
            assert!(
                !output.contains("Running full pipeline from keyboard shortcut."),
                "a queued Ctrl-G must not escalate the retried action:\n{output}"
            );
        },
    );
}

#[test]
fn piped_ctrl_r_after_a_hot_reload_rejects_the_stale_revision() {
    let config_template = |command: &str| {
        format!(
            r#"on:
  watch_backend: poll
  poll_interval: 20ms
jobs:
  - name: fails
    run: '{command}'
    change: 'src/**'
"#
        )
    };
    setup::with_config(
        std::path::Path::new("examples/retry-keys.yaml"),
        "shortcut_ctrl_r_reload.log",
        |fzz_cmd, mut output_log, fixture| {
            std::fs::write(
                fixture.join("examples/retry-keys.yaml"),
                config_template(
                    "echo fail >> fails.log; if test -f allow; then exit 0; else exit 1; fi",
                ),
            )
            .expect("write initial config");
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
                watcher_output(&mut output_log).contains("Watching..."),
                "watcher did not become ready"
            );
            trigger_change(fixture);
            wait_until!(
                read_fixture_log(fixture, "fails.log") == 1,
                "source run did not complete"
            );

            // Hot reload keeps the task name but changes the committed
            // revision; the recorded outcome is now stale.
            std::fs::write(
                fixture.join("examples/retry-keys.yaml"),
                config_template("echo reloaded >> fails.log"),
            )
            .expect("write reloaded config");
            wait_until!(
                watcher_output(&mut output_log).contains("hot-reloading to revision 2."),
                "watcher did not hot-reload to revision 2"
            );

            std::fs::write(fixture.join("allow"), "passes now").expect("allow retry");
            press(&mut input, CTRL_R);
            wait_until!(
                watcher_output(&mut output_log).contains(
                    "Shortcut ignored: config changed after the latest run; trigger a fresh run first.",
                ),
                "keyboard retry did not reject the stale revision"
            );

            assert_eq!(
                read_fixture_log(fixture, "fails.log"),
                1,
                "stale retry must not schedule work under the new revision"
            );
        },
    );
}
