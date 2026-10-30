#![cfg(all(feature = "test-integration", unix))]

use std::fs::File;
use std::io::{Read, Seek, SeekFrom};
use std::process::{Command, Stdio};

#[path = "./common/lib.rs"]
mod setup;

fn read_log(output_log: &mut File) -> String {
    let mut output = String::new();
    output_log.seek(SeekFrom::Start(0)).unwrap();
    output_log.read_to_string(&mut output).unwrap();
    output
}

fn read_fixture_file(fixture: &std::path::Path, name: &str) -> String {
    std::fs::read_to_string(fixture.join(name)).unwrap_or_default()
}

fn trigger_change(fixture: &std::path::Path, name: &str) {
    std::fs::write(fixture.join("src").join(name), "trigger").expect("write trigger file");
}

/// TASK-0194: two jobs with distinct configured `cwd`s both receive
/// `FZZ_TRIGGER_RELATIVE_PATH` as data with different values — computed
/// per job against its own resolved cwd. The root job sees workspace-root-
/// relative; the nested job has its cwd outside the trigger path and sees
/// the documented absolute-fallback (same as {{relative_filepath}}).
#[test]
fn blocking_watch_exposes_different_relative_path_per_job_cwd() {
    let config = r#"on:
  watch_backend: poll
  poll_interval: 20ms
jobs:
  - name: root-cwd-writer
    run: 'printf "%s" "$FZZ_TRIGGER_RELATIVE_PATH" > rel-root.txt'
    change: 'src/**'
  - name: nested-cwd-writer
    cwd: src/nested
    run: 'printf "%s" "$FZZ_TRIGGER_RELATIVE_PATH" > ../../rel-nested.txt'
    change: 'src/**'
"#;
    setup::with_config(
        std::path::Path::new("examples/simple-case.yml"),
        "trigger_rel_blocking.log",
        |fzz_cmd, mut output_log, fixture| {
            std::fs::create_dir_all(fixture.join("src/nested")).expect("create job cwd");
            std::fs::write(fixture.join("examples/simple-case.yml"), config).expect("write config");
            let mut child = fzz_cmd
                .stdin(Stdio::piped())
                .spawn()
                .expect("spawn watcher");
            let input = child.stdin.take().expect("watcher stdin");
            defer!({
                let _ = child.kill();
                let _ = child.wait();
            });

            wait_until!(
                read_log(&mut output_log).contains("Watching..."),
                "watcher did not become ready"
            );
            trigger_change(fixture, "review.txt");
            wait_until!(
                !read_fixture_file(fixture, "rel-root.txt").is_empty(),
                "root job did not run"
            );
            wait_until!(
                !read_fixture_file(fixture, "rel-nested.txt").is_empty(),
                "nested job did not run"
            );

            assert_eq!(
                read_fixture_file(fixture, "rel-root.txt"),
                "src/review.txt",
                "root-cwd job: workspace-root-relative"
            );
            let nested_value = read_fixture_file(fixture, "rel-nested.txt");
            assert!(
                nested_value.ends_with("src/review.txt"),
                "nested-cwd job: outside-cwd trigger uses absolute-fallback: {nested_value}"
            );
            drop(input);
        },
    );
}

/// TASK-0194: same contract on restart-policy (non-block) watch.
#[test]
fn nonblock_watch_exposes_fzz_trigger_relative_path() {
    let config = r#"on:
  socket: "SOCKET_PLACEHOLDER"
  watch_backend: poll
  poll_interval: 20ms
jobs:
  - name: env-writer
    run: 'printf "%s" "$FZZ_TRIGGER_RELATIVE_PATH" > rel-out.txt'
    change: 'src/**'
"#;
    let socket = std::env::temp_dir().join(format!("fzz-trigger-rel-{}.sock", std::process::id()));
    let _ = std::fs::remove_file(&socket);
    setup::with_config(
        std::path::Path::new("examples/simple-case.yml"),
        "trigger_rel_nonblock.log",
        move |fzz_cmd, mut output_log, fixture| {
            std::fs::write(
                fixture.join("examples/simple-case.yml"),
                config.replace("SOCKET_PLACEHOLDER", &socket.display().to_string()),
            )
            .expect("write config");
            fzz_cmd.env("FUNZZY_NON_BLOCK", "true");
            let mut child = fzz_cmd
                .stdin(Stdio::piped())
                .spawn()
                .expect("spawn watcher");
            defer!({
                let _ = child.kill();
                let _ = child.wait();
                let _ = std::fs::remove_file(&socket);
            });

            wait_until!(
                read_log(&mut output_log).contains("Watching..."),
                "watcher did not become ready"
            );
            trigger_change(fixture, "nonblock rel.txt");
            wait_until!(
                !read_fixture_file(fixture, "rel-out.txt").is_empty(),
                "non-block relative-env job did not run"
            );
            assert!(
                read_fixture_file(fixture, "rel-out.txt").ends_with("src/nonblock rel.txt"),
                "unexpected FZZ_TRIGGER_RELATIVE_PATH: {}",
                read_fixture_file(fixture, "rel-out.txt")
            );
        },
    );
}

/// TASK-0194: pathless runs strip the relative env even with inherited
/// or configured stale values; triggered runs override and set it.
#[test]
fn pathless_runs_strip_fzz_trigger_relative_path_and_triggered_runs_override_it() {
    let config = r#"on:
  watch_backend: poll
  poll_interval: 20ms
execution:
  env:
    FZZ_TRIGGER_RELATIVE_PATH: stale-configured
jobs:
  - name: recorder
    run: 'printf "%s" "${FZZ_TRIGGER_RELATIVE_PATH:-unset}" > out.txt'
    change: 'src/**'
    run_on_init: true
"#;
    setup::with_config(
        std::path::Path::new("examples/simple-case.yml"),
        "trigger_rel_pathless.log",
        |fzz_cmd, _output_log, fixture| {
            std::fs::write(fixture.join("examples/simple-case.yml"), config).expect("write config");
            fzz_cmd.env("FZZ_TRIGGER_RELATIVE_PATH", "stale-inherited");
            let mut child = fzz_cmd
                .stdin(Stdio::piped())
                .spawn()
                .expect("spawn watcher");
            let input = child.stdin.take().expect("watcher stdin");
            defer!({
                let _ = child.kill();
                let _ = child.wait();
            });

            wait_until!(
                !read_fixture_file(fixture, "out.txt").is_empty(),
                "init run did not complete"
            );
            assert_eq!(
                read_fixture_file(fixture, "out.txt"),
                "unset",
                "pathless init: no FZZ_TRIGGER_RELATIVE_PATH despite stale values"
            );

            trigger_change(fixture, "real trigger.txt");
            wait_until!(
                read_fixture_file(fixture, "out.txt") != "unset",
                "triggered run did not complete"
            );
            assert!(
                read_fixture_file(fixture, "out.txt").ends_with("src/real trigger.txt"),
                "triggered run overrides the stale configured value: {}",
                read_fixture_file(fixture, "out.txt")
            );
            drop(input);
        },
    );
}

/// TASK-0194: a retry of a file-triggered run retains the original
/// relative path through FZZ_TRIGGER_RELATIVE_PATH.
#[test]
fn nonblock_retry_retains_the_original_relative_trigger_path() {
    let config = r#"on:
  socket: "SOCKET_PLACEHOLDER"
  watch_backend: poll
  poll_interval: 20ms
jobs:
  - name: flaky
    run: 'printf "%s\n" "$FZZ_TRIGGER_RELATIVE_PATH" >> attempts.log; if test -f allow; then exit 0; else exit 1; fi'
    change: 'src/**'
"#;
    let socket =
        std::env::temp_dir().join(format!("fzz-trigger-rel-retry-{}.sock", std::process::id()));
    let _ = std::fs::remove_file(&socket);
    setup::with_config(
        std::path::Path::new("examples/simple-case.yml"),
        "trigger_rel_retry.log",
        move |fzz_cmd, mut output_log, fixture| {
            std::fs::write(
                fixture.join("examples/simple-case.yml"),
                config.replace("SOCKET_PLACEHOLDER", &socket.display().to_string()),
            )
            .expect("write config");
            fzz_cmd.env("FUNZZY_NON_BLOCK", "true");
            let mut child = fzz_cmd
                .stdin(Stdio::piped())
                .spawn()
                .expect("spawn watcher");
            defer!({
                let _ = child.kill();
                let _ = child.wait();
                let _ = std::fs::remove_file(&socket);
            });

            wait_until!(
                read_log(&mut output_log).contains("Watching..."),
                "watcher did not become ready"
            );
            trigger_change(fixture, "retry rel.txt");
            wait_until!(
                read_fixture_file(fixture, "attempts.log").lines().count() == 1,
                "source run did not complete"
            );

            std::fs::write(fixture.join("allow"), "passes now").expect("allow retry");
            let retry = Command::new(env!("CARGO_BIN_EXE_fzz"))
                .current_dir(fixture)
                .env("_TEST_FUNZZY_COLORED", "0")
                .args([
                    "-c",
                    "examples/simple-case.yml",
                    "control",
                    "--format",
                    "toon",
                    "retry",
                ])
                .output()
                .expect("schedule retry");
            assert!(
                retry.status.success(),
                "retry failed: {}",
                String::from_utf8_lossy(&retry.stdout)
            );

            wait_until!(
                read_fixture_file(fixture, "attempts.log").lines().count() == 2,
                "retry did not complete"
            );

            let lines: Vec<String> = read_fixture_file(fixture, "attempts.log")
                .lines()
                .map(str::to_owned)
                .collect();
            assert_eq!(lines.len(), 2, "attempts: {lines:?}");
            assert_eq!(lines[0], lines[1], "retry must retain the relative path");
            assert!(
                lines[0].ends_with("src/retry rel.txt"),
                "unexpected relative path: {lines:?}"
            );
        },
    );
}
