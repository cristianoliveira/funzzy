//! Black-box coverage for the per-child `FZZ_TRIGGER_PATH` contract.

#![cfg(all(feature = "test-integration", unix))]

use std::fs::{self, File};
use std::io::{Read, Seek, SeekFrom, Write};
use std::os::unix::fs::PermissionsExt;
use std::path::{Path, PathBuf};
use std::process::Stdio;

#[path = "./common/lib.rs"]
mod setup;

const SPECIAL_TRIGGER: &str = "src/space ; $(touch injected) 'quoted' &.txt";
const STALE_PARENT: &str = "inherited-stale";

fn write_capture_script(fixture: &Path) {
    let scripts = fixture.join("scripts");
    fs::create_dir_all(&scripts).expect("create scripts directory");

    let script = scripts.join("capture-trigger-path.sh");
    fs::write(
        &script,
        r#"#!/bin/sh
set -eu
if [ "${FZZ_TRIGGER_PATH+x}" = x ]; then
    printf '%s\n' "$FZZ_TRIGGER_PATH" >> "$1"
else
    printf '%s\n' '<unset>' >> "$1"
fi
"#,
    )
    .expect("write capture script");

    let mut permissions = fs::metadata(&script)
        .expect("read script permissions")
        .permissions();
    permissions.set_mode(0o755);
    fs::set_permissions(script, permissions).expect("make script executable");
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

fn write_watch_config(fixture: &Path, job: &str) {
    write_config(fixture, job, false);
}

fn write_config(fixture: &Path, job: &str, with_socket: bool) {
    let socket = if with_socket {
        "  socket: control.sock\n"
    } else {
        ""
    };
    fs::write(
        fixture.join(".watch.yaml"),
        format!(
            "on:\n{socket}  change: 'src/**'\nexecution:\n  env:\n    FZZ_TRIGGER_PATH: execution-stale\njobs:\n{job}\n"
        ),
    )
    .expect("write watcher config");
}

fn assert_watch_ready(log: &mut File) {
    wait_until!(
        read_log(log).contains("Watching..."),
        "watcher did not start"
    );
}

fn assert_path_file(path: &Path, expected: &str, message: &str) {
    wait_until!(
        fs::read_to_string(path)
            .map(|content| content.lines().count() == 1)
            .unwrap_or(false),
        "{message}"
    );
    assert_eq!(fs::read_to_string(path).unwrap(), format!("{expected}\n"));
}

#[test]
fn shell_script_receives_special_trigger_path_in_both_watch_modes() {
    for (mode, non_blocking) in [("blocking", false), ("non-blocking", true)] {
        setup::with_config(
            Path::new(".watch.yaml"),
            match mode {
                "blocking" => "trigger_path_shell_blocking.log",
                _ => "trigger_path_shell_non_blocking.log",
            },
            |fzz_cmd, mut output_log, fixture| {
                write_capture_script(fixture);
                write_watch_config(
                    fixture,
                    "  - name: capture\n    run: './scripts/capture-trigger-path.sh shell-paths.txt'\n    change: 'src/**'\n    env:\n      FZZ_TRIGGER_PATH: task-stale",
                );

                fzz_cmd.env("FZZ_TRIGGER_PATH", STALE_PARENT);
                if non_blocking {
                    fzz_cmd.env("FUNZZY_NON_BLOCK", "true");
                }
                let mut child = fzz_cmd
                    .stdin(Stdio::piped())
                    .spawn()
                    .expect("start watcher");
                defer!({
                    let _ = child.kill();
                    let _ = child.wait();
                });

                assert_watch_ready(&mut output_log);
                let trigger = write_trigger(fixture, SPECIAL_TRIGGER);
                let expected = trigger.to_string_lossy().into_owned();
                let capture = fixture.join("shell-paths.txt");
                assert_path_file(
                    &capture,
                    &expected,
                    "shell script did not capture the trigger path",
                );
                assert!(
                    !fixture.join("injected").exists(),
                    "shell-significant path data must not execute as command text"
                );
            },
        );
    }
}

#[test]
fn direct_argv_child_receives_special_trigger_path_from_exec_stdin() {
    setup::with_output("trigger_path_argv.log", |fzz_cmd, _output_log, fixture| {
        let trigger = write_trigger(fixture, SPECIAL_TRIGGER);
        let expected = trigger.to_string_lossy().into_owned();
        let relative_input = format!("{SPECIAL_TRIGGER}\n");
        let input = fixture.join("argv-input.txt");
        fs::write(&input, relative_input).expect("write exec path input");

        let mut child = fzz_cmd
            .env("FZZ_TRIGGER_PATH", STALE_PARENT)
            .args([
                "exec",
                "--",
                "sh",
                "-c",
                "printf '%s\\n' \"$FZZ_TRIGGER_PATH\" > argv-path.txt",
            ])
            .stdin(Stdio::from(
                File::open(input).expect("open exec path input"),
            ))
            .spawn()
            .expect("start direct-argv watcher");
        defer!({
            let _ = child.kill();
            let _ = child.wait();
        });

        assert_path_file(
            &fixture.join("argv-path.txt"),
            &expected,
            "direct-argv child did not receive the trigger path",
        );
        assert!(
            !fixture.join("injected").exists(),
            "shell-significant path data must remain an environment value"
        );
    });
}

#[test]
fn init_run_removes_inherited_and_configured_stale_trigger_path() {
    setup::with_config(
        Path::new(".watch.yaml"),
        "trigger_path_init_stale.log",
        |fzz_cmd, _output_log, fixture| {
            write_capture_script(fixture);
            write_watch_config(
                fixture,
                "  - name: init\n    run: './scripts/capture-trigger-path.sh init-path.txt'\n    change: 'src/**'\n    run_on_init: true\n    env:\n      FZZ_TRIGGER_PATH: task-stale",
            );
            fzz_cmd.env("FZZ_TRIGGER_PATH", STALE_PARENT);

            let mut child = fzz_cmd.spawn().expect("start init watcher");
            defer!({
                let _ = child.kill();
                let _ = child.wait();
            });

            assert_path_file(
                &fixture.join("init-path.txt"),
                "<unset>",
                "init job inherited a stale trigger path",
            );
        },
    );
}

#[test]
fn manual_target_removes_inherited_and_configured_stale_trigger_path() {
    setup::with_config(
        Path::new(".watch.yaml"),
        "trigger_path_manual_stale.log",
        |fzz_cmd, _output_log, fixture| {
            write_capture_script(fixture);
            write_watch_config(
                fixture,
                "  - name: manual\n    trigger: manual\n    run: './scripts/capture-trigger-path.sh manual-path.txt'\n    env:\n      FZZ_TRIGGER_PATH: task-stale",
            );
            fzz_cmd
                .env("FZZ_TRIGGER_PATH", STALE_PARENT)
                .args(["run", "manual"]);

            let status = fzz_cmd.status().expect("run manual target");
            assert!(status.success(), "manual target should succeed");
            assert_eq!(
                fs::read_to_string(fixture.join("manual-path.txt")).unwrap(),
                "<unset>\n"
            );
        },
    );
}

#[test]
fn ctrl_g_run_removes_stale_trigger_path_in_both_watch_modes() {
    for (mode, non_blocking) in [("blocking", false), ("non-blocking", true)] {
        setup::with_config(
            Path::new(".watch.yaml"),
            match mode {
                "blocking" => "trigger_path_ctrl_g_blocking.log",
                _ => "trigger_path_ctrl_g_non_blocking.log",
            },
            |fzz_cmd, mut output_log, fixture| {
                write_capture_script(fixture);
                write_watch_config(
                    fixture,
                    "  - name: manual\n    run: './scripts/capture-trigger-path.sh ctrl-g-path.txt'\n    change: 'src/**'\n    env:\n      FZZ_TRIGGER_PATH: task-stale",
                );
                fzz_cmd.env("FZZ_TRIGGER_PATH", STALE_PARENT);
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
                assert_path_file(
                    &fixture.join("ctrl-g-path.txt"),
                    "<unset>",
                    "Ctrl-G run inherited a stale trigger path",
                );
            },
        );
    }
}

#[test]
fn retry_keeps_original_trigger_path_in_both_watch_modes() {
    for (mode, non_blocking) in [("blocking", false), ("non-blocking", true)] {
        setup::with_config(
            Path::new(".watch.yaml"),
            match mode {
                "blocking" => "trigger_path_retry_blocking.log",
                _ => "trigger_path_retry_non_blocking.log",
            },
            |fzz_cmd, mut output_log, fixture| {
                write_config(
                    fixture,
                    "  - name: capture\n    run: 'printf \"%s\\n\" \"$FZZ_TRIGGER_PATH\" >> retry-paths.txt; test -f allow-retry'\n    change: 'src/**'\n    env:\n      FZZ_TRIGGER_PATH: task-stale",
                    true,
                );
                fzz_cmd.env("FZZ_TRIGGER_PATH", STALE_PARENT);
                if non_blocking {
                    fzz_cmd.env("FUNZZY_NON_BLOCK", "true");
                }

                let mut child = fzz_cmd.spawn().expect("start watcher");
                defer!({
                    let _ = child.kill();
                    let _ = child.wait();
                });

                wait_until!(
                    fixture.join("control.sock").exists(),
                    "retry control socket did not start"
                );
                let trigger = write_trigger(fixture, "src/retry-original.txt");
                let expected = trigger.to_string_lossy().into_owned();
                let retry_paths = fixture.join("retry-paths.txt");
                wait_until!(
                    fs::read_to_string(&retry_paths)
                        .map(|paths| paths.lines().count() == 1)
                        .unwrap_or(false)
                        && read_log(&mut output_log).contains("has failed with exit status: 1"),
                    "source generation did not fail"
                );

                fs::write(fixture.join("allow-retry"), "retry passes")
                    .expect("allow the retry to pass");
                let retry = std::process::Command::new(env!("CARGO_BIN_EXE_fzz"))
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
                    .expect("request retry over the control socket");
                assert!(
                    retry.status.success(),
                    "control retry failed: {}{}",
                    String::from_utf8_lossy(&retry.stdout),
                    String::from_utf8_lossy(&retry.stderr)
                );
                assert_eq!(
                    fs::read_to_string(&retry_paths).unwrap().lines().count(),
                    2,
                    "retry did not run"
                );
                assert_eq!(
                    fs::read_to_string(&retry_paths).unwrap(),
                    format!("{expected}\n{expected}\n"),
                    "retry must retain the original trigger path"
                );
            },
        );
    }
}

#[test]
fn legacy_absolute_path_templates_keep_their_existing_values() {
    setup::with_config(
        Path::new(".watch.yaml"),
        "trigger_path_legacy_templates.log",
        |fzz_cmd, mut output_log, fixture| {
            write_watch_config(
                fixture,
                "  - name: legacy\n    run: 'printf \"%s\\n%s\\n\" \"{{filepath}}\" \"{{absolute_path}}\" > legacy-paths.txt'\n    change: 'src/**'",
            );
            let mut child = fzz_cmd
                .stdin(Stdio::piped())
                .spawn()
                .expect("start legacy-template watcher");
            defer!({
                let _ = child.kill();
                let _ = child.wait();
            });

            assert_watch_ready(&mut output_log);
            let trigger = write_trigger(fixture, "src/legacy.txt");
            let expected = trigger.to_string_lossy().into_owned();
            wait_until!(
                fs::read_to_string(fixture.join("legacy-paths.txt"))
                    .map(|paths| paths.lines().count() == 2)
                    .unwrap_or(false),
                "legacy templates did not execute"
            );
            assert_eq!(
                fs::read_to_string(fixture.join("legacy-paths.txt")).unwrap(),
                format!("{expected}\n{expected}\n")
            );
        },
    );
}
