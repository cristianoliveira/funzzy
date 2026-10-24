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

// ---- QA (Jet) coverage, unioned for verification ----
const CTRL_G_BYTE: u8 = 0x07;
const CTRL_R_BYTE: u8 = 0x12;
const NO_PRIOR_WARNING: &str = "Shortcut ignored: no previous run to retry.";
const NO_RETRYABLE_WARNING: &str =
    "Shortcut ignored: latest run has no failed or timed-out jobs to retry.";

fn read_log(output_log: &mut std::fs::File) -> String {
    let mut output = String::new();
    output_log.seek(SeekFrom::Start(0)).unwrap();
    output_log.read_to_string(&mut output).unwrap();
    output
}

fn jet_write_retry_config(fixture: &std::path::Path, socket: Option<&std::path::Path>) {
    let socket = socket.map_or_else(String::new, |path| {
        format!("  socket: \"{}\"\n", path.display())
    });
    let config = format!(
        r#"on:
{socket}  watch_backend: poll
  poll_interval: 20ms
jobs:
  - name: failing finite job
    change: 'src/retry-*.txt'
    run: 'printf "%s\n" "{{{{filepath}}}}" >> retry-paths.log; printf "run\n" >> failed-runs.log; if test -f retry-ok; then exit 0; else exit 1; fi'
  - name: timed-out finite job
    change: 'src/retry-*.txt'
    timeout: 500ms
    run: 'printf "run\n" >> timedout-runs.log; if test -f retry-ok; then touch retry-complete; else sleep 30; fi'
  - name: passed finite job
    change: 'src/retry-*.txt'
    run: 'printf "run\n" >> passed-runs.log'
"#
    );
    std::fs::write(fixture.join("examples/simple-case.yml"), config)
        .expect("write shortcut retry config");
}

fn write_revision_retry_config(fixture: &std::path::Path, command: &str) {
    let config = format!(
        r#"on:
  watch_backend: poll
  poll_interval: 20ms
jobs:
  - name: stable task name
    change: 'src/retry-*.txt'
    run: '{command}'
"#
    );
    std::fs::write(fixture.join("examples/simple-case.yml"), config)
        .expect("write revision retry config");
}

fn run_count(fixture: &std::path::Path, name: &str) -> usize {
    std::fs::read_to_string(fixture.join(name))
        .unwrap_or_default()
        .lines()
        .count()
}

fn assert_retry_kept_filepath(fixture: &std::path::Path) {
    let paths: Vec<_> = std::fs::read_to_string(fixture.join("retry-paths.log"))
        .expect("retry path log")
        .lines()
        .map(str::to_owned)
        .collect();
    assert_eq!(
        paths.len(),
        3,
        "source, retry, and latest pass should record paths"
    );
    assert_eq!(paths[0], paths[1], "retry must keep the source filepath");
    assert!(
        std::path::Path::new(&paths[0]).ends_with("src/retry-first.txt"),
        "unexpected expanded source filepath: {:?}",
        paths[0]
    );
    assert!(
        std::path::Path::new(&paths[2]).ends_with("src/retry-latest-pass.txt"),
        "unexpected expanded filepath for the latest pass: {:?}",
        paths[2]
    );
}

fn wait_for_state(socket: &std::path::Path, generation: u64, expected: &str) -> serde_json::Value {
    wait_until!(
        status(socket).is_some_and(|value| {
            value["result"]["generation"].as_u64() == Some(generation)
                && value["result"]["state"] == expected
        }),
        "generation {generation} did not reach {expected}"
    );
    status(socket).expect("status snapshot")
}

#[test]
fn ctrl_r_retries_only_failed_and_timed_out_jobs_in_blocking_mode() {
    setup::with_config(
        std::path::Path::new("examples/simple-case.yml"),
        "shortcut_retry_blocking.log",
        |fzz_cmd, mut output_log, fixture| {
            jet_write_retry_config(fixture, None);
            let mut child = fzz_cmd
                .stdin(Stdio::piped())
                .spawn()
                .expect("failed to spawn watcher");
            let mut input = child.stdin.take().expect("watcher stdin");
            defer!({
                let _ = child.kill();
                let _ = child.wait();
            });

            wait_until!(read_log(&mut output_log).contains("Watching..."));
            input
                .write_all(&[CTRL_R_BYTE])
                .expect("write Ctrl+R without a source");
            wait_until!(
                read_log(&mut output_log).contains(NO_PRIOR_WARNING),
                "blocking Ctrl+R without a source was not rejected"
            );
            std::fs::write(fixture.join("src/retry-first.txt"), "trigger")
                .expect("write first trigger");
            wait_until!(
                run_count(fixture, "passed-runs.log") == 1,
                "initial blocking generation did not finish"
            );
            wait_until!(
                read_log(&mut output_log).contains("timedout"),
                "initial generation did not record the timeout"
            );

            std::fs::write(fixture.join("retry-ok"), "allow retry")
                .expect("allow failed finite jobs to pass on retry");
            input.write_all(&[CTRL_R_BYTE]).expect("write Ctrl+R");

            wait_until!(
                fixture.join("retry-complete").exists(),
                "Ctrl+R did not retry the failed and timed-out jobs"
            );
            std::fs::write(fixture.join("src/retry-latest-pass.txt"), "trigger")
                .expect("write latest passing trigger");
            wait_until!(
                run_count(fixture, "passed-runs.log") == 2,
                "latest blocking generation did not finish"
            );
            input
                .write_all(&[CTRL_R_BYTE])
                .expect("write Ctrl+R after pass");
            wait_until!(
                read_log(&mut output_log).contains(NO_RETRYABLE_WARNING),
                "blocking Ctrl+R replayed an older failed generation"
            );
            drop(input);
            assert_eq!(run_count(fixture, "failed-runs.log"), 3);
            assert_eq!(run_count(fixture, "timedout-runs.log"), 3);
            assert_eq!(run_count(fixture, "passed-runs.log"), 2);
            assert_retry_kept_filepath(fixture);
        },
    );
}

#[test]
fn ctrl_r_retries_only_failed_and_timed_out_jobs_in_socket_non_block_mode() {
    setup::with_config(
        std::path::Path::new("examples/simple-case.yml"),
        "shortcut_retry_non_block.log",
        |fzz_cmd, mut output_log, fixture| {
            let socket =
                std::env::temp_dir().join(format!("fzz-kbd-retry-{}.sock", std::process::id()));
            let _ = std::fs::remove_file(&socket);
            jet_write_retry_config(fixture, Some(&socket));
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
                status(&socket).is_some(),
                "control socket did not become ready"
            );
            std::fs::write(fixture.join("src/retry-first.txt"), "trigger")
                .expect("write first trigger");
            let source = wait_for_state(&socket, 1, "failed");
            assert_eq!(source["result"]["tasks"][0]["state"], "failed");
            assert_eq!(source["result"]["tasks"][1]["state"], "timedout");
            assert_eq!(source["result"]["tasks"][2]["state"], "passed");

            std::fs::write(fixture.join("retry-ok"), "allow retry")
                .expect("allow failed finite jobs to pass on retry");
            input.write_all(&[CTRL_R_BYTE]).expect("write Ctrl+R");

            let retried = wait_for_state(&socket, 2, "passed");
            let tasks = retried["result"]["tasks"].as_array().unwrap();
            assert_eq!(tasks.len(), 2, "retry should exclude the passed job");
            assert_eq!(tasks[0]["name"], "failing finite job");
            assert_eq!(tasks[1]["name"], "timed-out finite job");

            std::fs::write(fixture.join("src/retry-latest-pass.txt"), "trigger")
                .expect("write latest passing trigger");
            let latest_pass = wait_for_state(&socket, 3, "passed");
            assert_eq!(latest_pass["result"]["tasks"].as_array().unwrap().len(), 3);

            input
                .write_all(&[CTRL_R_BYTE])
                .expect("write Ctrl+R after pass");
            wait_until!(
                read_log(&mut output_log).contains(NO_RETRYABLE_WARNING),
                "non-block Ctrl+R replayed an older failed generation"
            );
            drop(input);
            assert_eq!(status(&socket).unwrap()["result"]["generation"], 3);
            assert_eq!(run_count(fixture, "failed-runs.log"), 3);
            assert_eq!(run_count(fixture, "timedout-runs.log"), 3);
            assert_eq!(run_count(fixture, "passed-runs.log"), 2);
            assert_retry_kept_filepath(fixture);
        },
    );
}

#[test]
fn busy_ctrl_r_wins_over_a_later_ctrl_g() {
    setup::with_config(
        std::path::Path::new("examples/simple-case.yml"),
        "shortcut_retry_busy.log",
        |fzz_cmd, mut output_log, fixture| {
            let socket =
                std::env::temp_dir().join(format!("fzz-kbd-busy-{}.sock", std::process::id()));
            let _ = std::fs::remove_file(&socket);
            std::fs::write(
                fixture.join("examples/simple-case.yml"),
                format!(
                    r#"on:
  socket: "{}"
  watch_backend: poll
  poll_interval: 20ms
jobs:
  - name: busy failure
    change: 'src/busy-*.txt'
    run_on_init: true
    run: 'printf "run\n" >> busy-failures.log; touch busy-started; while [ ! -f busy-release ]; do sleep 0.02; done; exit 1'
  - name: successful job
    change: 'src/busy-*.txt'
    run_on_init: true
    run: 'printf "run\n" >> busy-passes.log'
"#,
                    socket.display()
                ),
            )
            .expect("write busy shortcut config");
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
                status(&socket).is_some(),
                "control socket did not become ready"
            );
            wait_until!(fixture.join("busy-started").exists());
            let active = status(&socket).expect("active status");
            let generation = active["result"]["generation"].as_u64().unwrap();
            assert_eq!(active["result"]["state"], "running");

            input
                .write_all(&[CTRL_R_BYTE, CTRL_G_BYTE])
                .expect("write busy Ctrl+R and Ctrl+G");
            wait_until!(
                read_log(&mut output_log).contains("Shortcut latched"),
                "busy shortcut was not acknowledged"
            );
            std::fs::write(fixture.join("busy-release"), "release")
                .expect("release initial busy job");

            let retried = wait_for_state(&socket, generation + 1, "failed");
            assert_eq!(retried["result"]["tasks"].as_array().unwrap().len(), 1);
            assert_eq!(retried["result"]["tasks"][0]["name"], "busy failure");
            assert_eq!(run_count(fixture, "busy-failures.log"), 2);
            assert_eq!(run_count(fixture, "busy-passes.log"), 1);
        },
    );
}

#[test]
fn blocking_ctrl_r_wins_over_a_queued_ctrl_g() {
    setup::with_config(
        std::path::Path::new("examples/simple-case.yml"),
        "shortcut_retry_blocking_busy.log",
        |fzz_cmd, mut output_log, fixture| {
            std::fs::write(
                fixture.join("examples/simple-case.yml"),
                r#"on:
  watch_backend: poll
  poll_interval: 20ms
jobs:
  - name: slow failure
    change: 'src/retry-first.txt'
    run: 'printf "run\n" >> slow-runs.log; if test -f first-run; then sleep 1; exit 0; else touch first-run; sleep 1; exit 1; fi'
  - name: barrier marker
    change: 'src/barrier.txt'
    run: 'if test -n "{{filepath}}"; then echo event >> barrier-runs.log; else echo manual >> barrier-runs.log; fi'
"#,
            )
            .expect("write blocking busy shortcut config");
            let mut child = fzz_cmd
                .stdin(Stdio::piped())
                .spawn()
                .expect("failed to spawn watcher");
            let mut input = child.stdin.take().expect("watcher stdin");
            defer!({
                let _ = child.kill();
                let _ = child.wait();
            });

            wait_until!(read_log(&mut output_log).contains("Watching..."));
            std::fs::write(fixture.join("src/retry-first.txt"), "trigger")
                .expect("write failed source trigger");
            wait_until!(run_count(fixture, "slow-runs.log") == 1);
            std::fs::write(fixture.join("src/barrier.txt"), "barrier")
                .expect("write post-shortcut barrier");
            input
                .write_all(&[CTRL_R_BYTE, CTRL_G_BYTE])
                .expect("queue Ctrl+R then Ctrl+G while blocking run is active");

            wait_until!(
                std::fs::read_to_string(fixture.join("barrier-runs.log"))
                    .is_ok_and(|output| output.lines().any(|line| line == "event")),
                "filesystem barrier did not run after queued shortcuts"
            );
            assert_eq!(run_count(fixture, "slow-runs.log"), 2);
            assert_eq!(
                std::fs::read_to_string(fixture.join("barrier-runs.log")).unwrap(),
                "event\n",
                "queued Ctrl-G must not trigger a full run before the barrier event"
            );
            assert!(!read_log(&mut output_log)
                .contains("Running full pipeline from keyboard shortcut."));
        },
    );
}

#[test]
fn ctrl_r_without_a_prior_generation_does_not_run_any_jobs() {
    setup::with_config(
        std::path::Path::new("examples/simple-case.yml"),
        "shortcut_retry_without_source.log",
        |fzz_cmd, mut output_log, fixture| {
            let socket =
                std::env::temp_dir().join(format!("fzz-kbd-no-source-{}.sock", std::process::id()));
            let _ = std::fs::remove_file(&socket);
            std::fs::write(
                fixture.join("examples/simple-case.yml"),
                format!(
                    r#"on:
  socket: "{}"
  watch_backend: poll
  poll_interval: 20ms
jobs:
  - name: trigger-only job
    change: 'src/no-prior.txt'
    run: 'printf "run\n" >> trigger-runs.log'
  - name: full-pipeline-only job
    change: 'src/manual-only.txt'
    run: 'printf "run\n" >> manual-only-runs.log'
"#,
                    socket.display()
                ),
            )
            .expect("write no-source config");
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
                status(&socket).is_some(),
                "control socket did not become ready"
            );
            assert_eq!(status(&socket).unwrap()["result"]["generation"], 0);
            input.write_all(&[CTRL_R_BYTE]).expect("write Ctrl+R");
            wait_until!(
                read_log(&mut output_log).contains(NO_PRIOR_WARNING),
                "non-block Ctrl+R without a source was not rejected"
            );
            std::fs::write(fixture.join("src/no-prior.txt"), "trigger")
                .expect("write a single-task filesystem trigger");
            drop(input);

            let completed = wait_for_state(&socket, 1, "passed");
            assert_eq!(completed["result"]["tasks"].as_array().unwrap().len(), 1);
            assert_eq!(run_count(fixture, "trigger-runs.log"), 1);
            assert_eq!(run_count(fixture, "manual-only-runs.log"), 0);
        },
    );
}

#[test]
fn ctrl_r_rejects_a_same_name_job_after_config_revision_changes() {
    setup::with_config(
        std::path::Path::new("examples/simple-case.yml"),
        "shortcut_retry_stale_revision.log",
        |fzz_cmd, mut output_log, fixture| {
            write_revision_retry_config(fixture, "echo old >> old-runs.log; exit 1");
            let mut child = fzz_cmd
                .stdin(Stdio::piped())
                .spawn()
                .expect("failed to spawn watcher");
            let mut input = child.stdin.take().expect("watcher stdin");
            defer!({
                let _ = child.kill();
                let _ = child.wait();
            });

            wait_until!(read_log(&mut output_log).contains("Watching..."));
            std::fs::write(fixture.join("src/retry-first.txt"), "trigger")
                .expect("write first trigger");
            wait_until!(run_count(fixture, "old-runs.log") == 1);

            write_revision_retry_config(fixture, "echo new >> new-runs.log; exit 1");
            wait_until!(
                read_log(&mut output_log)
                    .contains("Config change is valid; hot-reloading to revision 2."),
                "same-name semantic config change did not reload"
            );

            input.write_all(&[CTRL_R_BYTE]).expect("write Ctrl+R");
            let stale_warning =
                "Shortcut ignored: config changed after the latest run; trigger a fresh run first.";
            wait_until!(
                fixture.join("new-runs.log").exists()
                    || read_log(&mut output_log).contains(stale_warning),
                "retry neither ran nor rejected after reload"
            );
            assert!(
                !fixture.join("new-runs.log").exists(),
                "Ctrl+R executed a same-name job from the newer config revision"
            );
            assert!(read_log(&mut output_log).contains(stale_warning));
        },
    );
}

#[test]
fn ctrl_r_service_rejection_does_not_schedule_partial_work() {
    setup::with_config(
        std::path::Path::new("examples/simple-case.yml"),
        "shortcut_retry_service.log",
        |fzz_cmd, mut output_log, fixture| {
            let socket =
                std::env::temp_dir().join(format!("fzz-kbd-service-{}.sock", std::process::id()));
            let _ = std::fs::remove_file(&socket);
            std::fs::write(
                fixture.join("examples/simple-case.yml"),
                format!(
                    r#"on:
  socket: "{}"
  watch_backend: poll
  poll_interval: 20ms
jobs:
  - name: finite failure
    change: 'src/service-trigger.txt'
    run: 'printf "run\n" >> finite-runs.log; exit 1'
  - name: unsupported service
    service: true
    change: 'src/service-trigger.txt'
    run: 'printf "run\n" >> service-runs.log; exit 1'
"#,
                    socket.display()
                ),
            )
            .expect("write service config");
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
                status(&socket).is_some(),
                "control socket did not become ready"
            );
            std::fs::write(fixture.join("src/service-trigger.txt"), "trigger")
                .expect("write service trigger");
            let source = wait_for_state(&socket, 1, "failed");
            assert_eq!(source["result"]["tasks"].as_array().unwrap().len(), 2);

            input.write_all(&[CTRL_R_BYTE]).expect("write Ctrl+R");
            wait_until!(
                read_log(&mut output_log).contains(
                    "Shortcut ignored: service job 'unsupported service' cannot be retried."
                ),
                "service retry rejection was not reported"
            );
            assert_eq!(run_count(fixture, "finite-runs.log"), 1);
            assert_eq!(status(&socket).unwrap()["result"]["generation"], 1);

            input.write_all(&[CTRL_G_BYTE]).expect("write Ctrl+G");
            drop(input);
            let full_run = wait_for_state(&socket, 2, "failed");
            assert_eq!(full_run["result"]["trigger"], "keyboard");
            assert_eq!(full_run["result"]["tasks"].as_array().unwrap().len(), 2);
            assert_eq!(run_count(fixture, "finite-runs.log"), 2);
        },
    );
}
