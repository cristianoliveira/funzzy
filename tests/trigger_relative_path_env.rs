//! Black-box coverage for the per-job-cwd `FZZ_TRIGGER_RELATIVE_PATH` contract.

#![cfg(all(feature = "test-integration", unix))]

use std::fs::{self, File};
use std::io::{Read, Seek, SeekFrom, Write};
use std::os::unix::fs::PermissionsExt;
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};

#[path = "./common/lib.rs"]
mod setup;

const RELATIVE_PATH_ENV: &str = "FZZ_TRIGGER_RELATIVE_PATH";
const STALE_PARENT: &str = "inherited-relative-stale";
const SPECIAL_TRIGGER: &str = "src/space ; $(touch injected) 'quoted' &.txt";

fn write_capture_script(fixture: &Path) {
    let scripts = fixture.join("scripts");
    fs::create_dir_all(&scripts).expect("create scripts directory");

    let script = scripts.join("capture-relative-path.sh");
    fs::write(
        &script,
        r#"#!/bin/sh
set -eu
output=$1
shift
if [ "${FZZ_TRIGGER_RELATIVE_PATH+x}" = x ]; then
    printf '%s\n' "$FZZ_TRIGGER_RELATIVE_PATH" > "$output"
else
    printf '%s\n' '<unset>' > "$output"
fi
for legacy_path in "$@"; do
    printf '%s\n' "$legacy_path" >> "$output"
done
"#,
    )
    .expect("write capture script");

    let mut permissions = fs::metadata(&script)
        .expect("read script permissions")
        .permissions();
    permissions.set_mode(0o755);
    fs::set_permissions(script, permissions).expect("make capture script executable");
}

fn write_trigger(fixture: &Path, relative_path: &str) -> PathBuf {
    let trigger = fixture.join(relative_path);
    fs::create_dir_all(trigger.parent().expect("trigger parent"))
        .expect("create trigger directory");
    fs::write(&trigger, "changed").expect("write trigger file");
    trigger
}

fn read_log(log: &mut File) -> String {
    log.seek(SeekFrom::Start(0)).expect("rewind watcher log");
    let mut contents = String::new();
    log.read_to_string(&mut contents).expect("read watcher log");
    contents
}

fn wait_for_lines(path: &Path, expected: usize, message: &str) {
    wait_until!(
        fs::read_to_string(path)
            .map(|contents| contents.lines().count() == expected)
            .unwrap_or(false),
        "{message}"
    );
}

fn assert_contents(path: &Path, expected: &str) {
    assert_eq!(
        fs::read_to_string(path).unwrap(),
        expected,
        "{}",
        path.display()
    );
}

fn write_config(fixture: &Path, config: &str) {
    fs::write(fixture.join(".watch.yaml"), config).expect("write watcher config");
}

fn assert_watch_ready(log: &mut File) {
    wait_until!(
        read_log(log).contains("Watching..."),
        "watcher did not start"
    );
}

fn wait_for_result_count(log: &mut File, expected: usize, message: &str) {
    wait_until!(
        read_log(log).matches("Funzzy results").count() >= expected,
        "{message}"
    );
}

#[test]
fn shell_jobs_receive_the_relative_value_for_each_cwd_and_legacy_templates_stay_equal() {
    let config = r#"on:
  watch_backend: poll
  poll_interval: 20ms
jobs:
  - name: source-cwd
    cwd: src
    run: "../scripts/capture-relative-path.sh ../source-values.txt '{{relative_filepath}}' '{{relative_path}}'"
    change: 'src/**'
  - name: tests-cwd
    cwd: tests
    run: "../scripts/capture-relative-path.sh ../tests-values.txt '{{relative_filepath}}' '{{relative_path}}'"
    change: 'src/**'
"#;

    for (mode, non_blocking) in [("blocking", false), ("non-blocking", true)] {
        setup::with_config(
            Path::new(".watch.yaml"),
            match mode {
                "blocking" => "trigger_relative_cwd_blocking.log",
                _ => "trigger_relative_cwd_non_blocking.log",
            },
            |fzz_cmd, mut output_log, fixture| {
                write_capture_script(fixture);
                write_config(fixture, config);
                if non_blocking {
                    fzz_cmd.env("FUNZZY_NON_BLOCK", "true");
                }

                let mut child = fzz_cmd
                    .stdin(Stdio::piped())
                    .spawn()
                    .expect("start watcher");
                let input = child.stdin.take().expect("watcher stdin");
                defer!({
                    let _ = child.kill();
                    let _ = child.wait();
                });

                assert_watch_ready(&mut output_log);
                let trigger = write_trigger(fixture, "src/ordinary.txt");
                let outside_cwd = trigger.to_string_lossy().into_owned();

                let source_output = fixture.join("source-values.txt");
                wait_for_lines(
                    &source_output,
                    3,
                    "source-cwd job did not capture env and legacy values",
                );

                let tests_output = fixture.join("tests-values.txt");
                wait_for_lines(
                    &tests_output,
                    3,
                    "outside-cwd job did not capture env and legacy values",
                );
                wait_for_result_count(&mut output_log, 1, "two-cwd generation did not finish");
                assert_contents(&source_output, "ordinary.txt\nordinary.txt\nordinary.txt\n");
                assert_contents(
                    &tests_output,
                    &format!("{outside_cwd}\n{outside_cwd}\n{outside_cwd}\n"),
                );
                drop(input);
            },
        );
    }
}

#[test]
fn shell_child_receives_shell_significant_relative_path_as_literal_data_in_both_modes() {
    let config = r#"on:
  watch_backend: poll
  poll_interval: 20ms
jobs:
  - name: special-path
    cwd: src
    run: '../scripts/capture-relative-path.sh ../special-values.txt'
    change: 'src/**'
    env:
      FZZ_TRIGGER_RELATIVE_PATH: stale-job
"#;

    for (mode, non_blocking) in [("blocking", false), ("non-blocking", true)] {
        setup::with_config(
            Path::new(".watch.yaml"),
            match mode {
                "blocking" => "trigger_relative_special_blocking.log",
                _ => "trigger_relative_special_non_blocking.log",
            },
            |fzz_cmd, mut output_log, fixture| {
                write_capture_script(fixture);
                write_config(fixture, config);
                fzz_cmd.env(RELATIVE_PATH_ENV, STALE_PARENT);
                if non_blocking {
                    fzz_cmd.env("FUNZZY_NON_BLOCK", "true");
                }

                let mut child = fzz_cmd
                    .stdin(Stdio::piped())
                    .spawn()
                    .expect("start watcher");
                let input = child.stdin.take().expect("watcher stdin");
                defer!({
                    let _ = child.kill();
                    let _ = child.wait();
                });

                assert_watch_ready(&mut output_log);
                write_trigger(fixture, SPECIAL_TRIGGER);
                let output = fixture.join("special-values.txt");
                wait_for_lines(&output, 1, "special-path job did not run");
                wait_for_result_count(&mut output_log, 1, "special-path generation did not finish");
                assert_contents(&output, "space ; $(touch injected) 'quoted' &.txt\n");
                assert!(
                    !fixture.join("injected").exists(),
                    "path data must not execute as shell text"
                );
                drop(input);
            },
        );
    }
}

#[test]
fn pathless_init_removes_stale_values_and_triggered_run_overrides_them_in_both_modes() {
    let config = r#"on:
  watch_backend: poll
  poll_interval: 20ms
execution:
  env:
    FZZ_TRIGGER_RELATIVE_PATH: stale-execution
jobs:
  - name: recorder
    cwd: src
    run: '../scripts/capture-relative-path.sh ../init-values.txt'
    change: 'src/**'
    run_on_init: true
    env:
      FZZ_TRIGGER_RELATIVE_PATH: stale-job
"#;

    for (mode, non_blocking) in [("blocking", false), ("non-blocking", true)] {
        setup::with_config(
            Path::new(".watch.yaml"),
            match mode {
                "blocking" => "trigger_relative_init_blocking.log",
                _ => "trigger_relative_init_non_blocking.log",
            },
            |fzz_cmd, mut output_log, fixture| {
                write_capture_script(fixture);
                write_config(fixture, config);
                fzz_cmd.env(RELATIVE_PATH_ENV, STALE_PARENT);
                if non_blocking {
                    fzz_cmd.env("FUNZZY_NON_BLOCK", "true");
                }

                let mut child = fzz_cmd
                    .stdin(Stdio::piped())
                    .spawn()
                    .expect("start init watcher");
                let input = child.stdin.take().expect("watcher stdin");
                defer!({
                    let _ = child.kill();
                    let _ = child.wait();
                });

                let output = fixture.join("init-values.txt");
                wait_for_lines(&output, 1, "pathless init job did not run");
                wait_for_result_count(&mut output_log, 1, "init generation did not finish");
                assert_contents(&output, "<unset>\n");

                write_trigger(fixture, "src/actual.txt");
                wait_for_lines(&output, 2, "triggered job did not run after init");
                wait_for_result_count(&mut output_log, 2, "triggered generation did not finish");
                assert_contents(&output, "<unset>\nactual.txt\n");
                drop(input);
            },
        );
    }
}

#[test]
fn manual_target_removes_inherited_and_configured_stale_values() {
    let config = r#"execution:
  env:
    FZZ_TRIGGER_RELATIVE_PATH: stale-execution
jobs:
  - name: manual
    cwd: src
    trigger: manual
    run: '../scripts/capture-relative-path.sh ../manual-values.txt'
    env:
      FZZ_TRIGGER_RELATIVE_PATH: stale-job
"#;
    setup::with_config(
        Path::new(".watch.yaml"),
        "trigger_relative_manual.log",
        |fzz_cmd, _output_log, fixture| {
            write_capture_script(fixture);
            write_config(fixture, config);
            let output = fzz_cmd
                .env(RELATIVE_PATH_ENV, STALE_PARENT)
                .args(["run", "manual"])
                .output()
                .expect("run manual target");
            assert!(
                output.status.success(),
                "manual target failed: {}",
                String::from_utf8_lossy(&output.stderr)
            );
            assert_contents(&fixture.join("manual-values.txt"), "<unset>\n");
        },
    );
}

#[test]
fn ctrl_g_removes_inherited_and_configured_stale_values_in_both_modes() {
    let config = r#"on:
  watch_backend: poll
  poll_interval: 20ms
execution:
  env:
    FZZ_TRIGGER_RELATIVE_PATH: stale-execution
jobs:
  - name: keyboard
    cwd: src
    run: '../scripts/capture-relative-path.sh ../ctrl-g-values.txt'
    change: 'src/**'
    env:
      FZZ_TRIGGER_RELATIVE_PATH: stale-job
"#;
    for (mode, non_blocking) in [("blocking", false), ("non-blocking", true)] {
        setup::with_config(
            Path::new(".watch.yaml"),
            match mode {
                "blocking" => "trigger_relative_ctrl_g_blocking.log",
                _ => "trigger_relative_ctrl_g_non_blocking.log",
            },
            |fzz_cmd, mut output_log, fixture| {
                write_capture_script(fixture);
                write_config(fixture, config);
                fzz_cmd.env(RELATIVE_PATH_ENV, STALE_PARENT);
                if non_blocking {
                    fzz_cmd.env("FUNZZY_NON_BLOCK", "true");
                }

                let mut child = fzz_cmd
                    .stdin(Stdio::piped())
                    .spawn()
                    .expect("start watcher");
                let mut input = child.stdin.take().expect("watcher stdin");
                defer!({
                    let _ = child.kill();
                    let _ = child.wait();
                });

                assert_watch_ready(&mut output_log);
                input.write_all(&[0x07]).expect("send Ctrl-G");
                let output = fixture.join("ctrl-g-values.txt");
                wait_for_lines(&output, 1, "Ctrl-G job did not run");
                wait_for_result_count(&mut output_log, 1, "Ctrl-G generation did not finish");
                assert_contents(&output, "<unset>\n");
            },
        );
    }
}

#[test]
fn retries_keep_the_original_cwd_relative_trigger_path_in_both_watch_modes() {
    let config = r#"on:
  socket: control.sock
  watch_backend: poll
  poll_interval: 20ms
execution:
  env:
    FZZ_TRIGGER_RELATIVE_PATH: stale-execution
jobs:
  - name: flaky
    cwd: src
    run: 'printf "%s\n" "$FZZ_TRIGGER_RELATIVE_PATH" >> ../retry-values.txt; test -f ../allow-retry'
    change: 'src/**'
    env:
      FZZ_TRIGGER_RELATIVE_PATH: stale-job
"#;

    for (mode, non_blocking) in [("blocking", false), ("non-blocking", true)] {
        setup::with_config(
            Path::new(".watch.yaml"),
            match mode {
                "blocking" => "trigger_relative_retry_blocking.log",
                _ => "trigger_relative_retry_non_blocking.log",
            },
            |fzz_cmd, mut output_log, fixture| {
                write_config(fixture, config);
                fzz_cmd.env(RELATIVE_PATH_ENV, STALE_PARENT);
                if non_blocking {
                    fzz_cmd.env("FUNZZY_NON_BLOCK", "true");
                }

                let mut child = fzz_cmd.spawn().expect("start retry watcher");
                defer!({
                    let _ = child.kill();
                    let _ = child.wait();
                });

                wait_until!(
                    fixture.join("control.sock").exists(),
                    "retry control socket did not start"
                );
                write_trigger(fixture, "src/retry.txt");
                let retries = fixture.join("retry-values.txt");
                wait_until!(
                    fs::read_to_string(&retries)
                        .map(|values| values.lines().count() == 1)
                        .unwrap_or(false)
                        && read_log(&mut output_log).contains("has failed with exit status: 1"),
                    "source generation did not fail"
                );

                fs::write(fixture.join("allow-retry"), "retry passes").expect("allow retry");
                let retry = Command::new(env!("CARGO_BIN_EXE_fzz"))
                    .current_dir(fixture)
                    .args([
                        "control",
                        "--format",
                        "json",
                        "retry",
                        "--wait",
                        "--timeout",
                        "30s",
                    ])
                    .output()
                    .expect("request retry");
                assert!(
                    retry.status.success(),
                    "control retry failed: {}{}",
                    String::from_utf8_lossy(&retry.stdout),
                    String::from_utf8_lossy(&retry.stderr)
                );

                assert_contents(&retries, "retry.txt\nretry.txt\n");
            },
        );
    }
}

#[test]
fn direct_argv_child_receives_relative_path_as_data_and_strips_pathless_inheritance() {
    setup::with_output(
        "trigger_relative_argv.log",
        |fzz_cmd, mut output_log, fixture| {
            write_trigger(fixture, SPECIAL_TRIGGER);
            let input = fixture.join("argv-input.txt");
            fs::write(&input, format!("{SPECIAL_TRIGGER}\n")).expect("write exec path input");

            let mut child = fzz_cmd
            .env(RELATIVE_PATH_ENV, STALE_PARENT)
            .args([
                "exec",
                "--",
                "sh",
                "-c",
                "if [ \"${FZZ_TRIGGER_RELATIVE_PATH+x}\" = x ]; then printf '%s\\n' \"$FZZ_TRIGGER_RELATIVE_PATH\" >> argv-relative-values.txt; else printf '%s\\n' '<unset>' >> argv-relative-values.txt; fi",
            ])
            .stdin(Stdio::from(File::open(input).expect("open exec path input")))
            .spawn()
            .expect("start direct-argv watcher");
            defer!({
                let _ = child.kill();
                let _ = child.wait();
            });

            let captures = fixture.join("argv-relative-values.txt");
            wait_for_lines(&captures, 1, "exec init child did not run");
            wait_for_result_count(&mut output_log, 1, "exec init generation did not finish");
            assert_contents(&captures, "<unset>\n");

            let trigger = fixture.join(SPECIAL_TRIGGER);
            fs::write(&trigger, "changed again").expect("trigger an exec watch event");
            wait_for_lines(
                &captures,
                2,
                "direct-argv child did not run for the changed path",
            );
            wait_for_result_count(
                &mut output_log,
                2,
                "exec triggered generation did not finish",
            );
            assert_contents(&captures, &format!("<unset>\n{SPECIAL_TRIGGER}\n"));
            assert!(
                !fixture.join("injected").exists(),
                "shell-significant path data must remain an environment value"
            );
        },
    );
}
