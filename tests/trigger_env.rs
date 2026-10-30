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

/// TASK-0193 acceptance: the trigger path — including shell-significant
/// characters — reaches the child as data through FZZ_TRIGGER_PATH, byte
/// identical to `{{filepath}}`, and is never inserted into command text.
#[test]
fn blocking_watch_exposes_fzz_trigger_path_as_data() {
    let config = r#"on:
  watch_backend: poll
  poll_interval: 20ms
jobs:
  - name: env-writer
    run: 'printf "%s" "$FZZ_TRIGGER_PATH" > env-out.txt'
    change: 'src/**'
  - name: template-writer
    run: 'printf "%s" "{{filepath}}" > template-out.txt'
    change: 'src/**'
"#;
    setup::with_config(
        std::path::Path::new("examples/simple-case.yml"),
        "trigger_env_blocking.log",
        |fzz_cmd, mut output_log, fixture| {
            std::fs::write(fixture.join("examples/simple-case.yml"), config).expect("write config");
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
                read_log(&mut output_log).contains("Watching..."),
                "watcher did not become ready"
            );
            // Shell-significant characters: space, single quote, and an
            // unbound `$x`. The env value is data, so it stays byte-identical
            // to the reported path; the same path inserted into command text
            // by `{{filepath}}` gets `$x` eaten by the child shell — the
            // documented deprecation motivation.
            trigger_change(fixture, "we ird'$x.txt");
            wait_until!(
                !read_fixture_file(fixture, "env-out.txt").is_empty(),
                "env job did not run"
            );

            let env_value = read_fixture_file(fixture, "env-out.txt");
            assert!(
                env_value.ends_with("src/we ird'$x.txt"),
                "FZZ_TRIGGER_PATH must carry the raw path as data: {env_value}"
            );

            let template_value = read_fixture_file(fixture, "template-out.txt");
            assert!(
                template_value.contains("we ird'") && template_value.ends_with(".txt"),
                "{{filepath}} must stay functional: {template_value}"
            );
            assert_ne!(
                env_value, template_value,
                "path data must not be re-evaluated as shell text through the env"
            );
            drop(input);
        },
    );
}

/// TASK-0193 acceptance: the same contract on the restart-policy,
/// control-socket watch (non-blocking).
#[test]
fn nonblock_watch_exposes_fzz_trigger_path_as_data() {
    let config = r#"on:
  socket: "SOCKET_PLACEHOLDER"
  watch_backend: poll
  poll_interval: 20ms
jobs:
  - name: env-writer
    run: 'printf "%s" "$FZZ_TRIGGER_PATH" > env-out.txt'
    change: 'src/**'
"#;
    let socket = std::env::temp_dir().join(format!("fzz-trigger-env-{}.sock", std::process::id()));
    let _ = std::fs::remove_file(&socket);
    setup::with_config(
        std::path::Path::new("examples/simple-case.yml"),
        "trigger_env_nonblock.log",
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
                .expect("failed to spawn watcher");
            defer!({
                let _ = child.kill();
                let _ = child.wait();
                let _ = std::fs::remove_file(&socket);
            });

            wait_until!(
                read_log(&mut output_log).contains("Watching..."),
                "watcher did not become ready"
            );
            trigger_change(fixture, "nonblock trigger.txt");
            wait_until!(
                !read_fixture_file(fixture, "env-out.txt").is_empty(),
                "non-block env job did not run"
            );
            assert!(
                read_fixture_file(fixture, "env-out.txt").ends_with("nonblock trigger.txt"),
                "unexpected FZZ_TRIGGER_PATH: {}",
                read_fixture_file(fixture, "env-out.txt")
            );
        },
    );
}

/// TASK-0193 acceptance: a run with no trigger path has NO FZZ_TRIGGER_PATH —
/// not the parent's inherited value, not the configured `execution.env`
/// value — while a triggered run of the same job overrides the configured
/// stale value with the real path.
#[test]
fn pathless_runs_strip_fzz_trigger_path_and_triggered_runs_override_it() {
    let config = r#"on:
  watch_backend: poll
  poll_interval: 20ms
execution:
  env:
    FZZ_TRIGGER_PATH: stale-configured
jobs:
  - name: recorder
    run: 'printf "%s" "${FZZ_TRIGGER_PATH:-unset}" > out.txt'
    change: 'src/**'
    run_on_init: true
"#;
    setup::with_config(
        std::path::Path::new("examples/simple-case.yml"),
        "trigger_env_pathless.log",
        |fzz_cmd, mut output_log, fixture| {
            std::fs::write(fixture.join("examples/simple-case.yml"), config).expect("write config");
            // Stale value inherited by the Funzzy process itself.
            fzz_cmd.env("FZZ_TRIGGER_PATH", "stale-inherited");
            let mut child = fzz_cmd
                .stdin(Stdio::piped())
                .spawn()
                .expect("failed to spawn watcher");
            let mut input = child.stdin.take().expect("watcher stdin");
            defer!({
                let _ = child.kill();
                let _ = child.wait();
            });

            // A run_on_init watcher never prints "Watching..."; the init
            // completion itself is the readiness signal here.
            wait_until!(
                !read_fixture_file(fixture, "out.txt").is_empty(),
                "init run did not complete"
            );
            assert_eq!(
                read_fixture_file(fixture, "out.txt"),
                "unset",
                "a pathless init run must see no FZZ_TRIGGER_PATH despite \
                 inherited and configured stale values"
            );

            trigger_change(fixture, "real trigger.txt");
            wait_until!(
                read_fixture_file(fixture, "out.txt") != "unset",
                "triggered run did not complete"
            );
            assert!(
                read_fixture_file(fixture, "out.txt").ends_with("real trigger.txt"),
                "triggered run must override the configured stale value: {}",
                read_fixture_file(fixture, "out.txt")
            );
            drop(input);
        },
    );
}

/// TASK-0193 acceptance: a pathless finite manual target run has no
/// FZZ_TRIGGER_PATH either.
#[test]
fn finite_target_run_has_no_fzz_trigger_path() {
    let config = r#"jobs:
  - name: once
    trigger: manual
    run: 'printf "%s" "${FZZ_TRIGGER_PATH:-unset}" > out.txt'
"#;
    setup::with_config(
        std::path::Path::new("examples/simple-case.yml"),
        "trigger_env_finite.log",
        |_fzz_cmd, _output_log, fixture| {
            std::fs::write(fixture.join("examples/simple-case.yml"), config).expect("write config");

            let output = Command::new(env!("CARGO_BIN_EXE_fzz"))
                .current_dir(fixture)
                .env("FZZ_TRIGGER_PATH", "stale-inherited")
                .env("_TEST_FUNZZY_COLORED", "0")
                .args(["-c", "examples/simple-case.yml", "run", "once"])
                .output()
                .expect("run finite target");

            assert!(
                output.status.success(),
                "finite run failed: {:?}",
                String::from_utf8_lossy(&output.stderr)
            );
            assert_eq!(
                read_fixture_file(fixture, "out.txt"),
                "unset",
                "a finite target run must see no FZZ_TRIGGER_PATH"
            );
        },
    );
}

/// TASK-0193 acceptance: a retry of a file-triggered run retains its
/// original path through FZZ_TRIGGER_PATH.
#[test]
fn nonblock_retry_retains_the_original_trigger_path() {
    let config = r#"on:
  socket: "SOCKET_PLACEHOLDER"
  watch_backend: poll
  poll_interval: 20ms
jobs:
  - name: flaky
    run: 'printf "%s\n" "$FZZ_TRIGGER_PATH" >> attempts.log; if test -f allow; then exit 0; else exit 1; fi'
    change: 'src/**'
"#;
    let socket =
        std::env::temp_dir().join(format!("fzz-trigger-retry-{}.sock", std::process::id()));
    let _ = std::fs::remove_file(&socket);
    setup::with_config(
        std::path::Path::new("examples/simple-case.yml"),
        "trigger_env_retry.log",
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
                .expect("failed to spawn watcher");
            defer!({
                let _ = child.kill();
                let _ = child.wait();
                let _ = std::fs::remove_file(&socket);
            });

            wait_until!(
                read_log(&mut output_log).contains("Watching..."),
                "watcher did not become ready"
            );
            trigger_change(fixture, "retry first.txt");
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
            assert_eq!(lines[0], lines[1], "retry must retain the original path");
            assert!(
                lines[0].ends_with("retry first.txt"),
                "unexpected trigger path: {lines:?}"
            );
        },
    );
}
