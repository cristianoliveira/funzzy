//! Independent black-box acceptance for TASK-0196.
#![cfg(all(feature = "test-integration", unix))]

use std::fs::{self, File};
use std::io::{Read, Seek, SeekFrom, Write};
use std::path::Path;
use std::process::{Command, Stdio};

#[path = "./common/lib.rs"]
mod setup;

const ENV: &str = "FZZ_TRIGGER_PATHS_FILE";
const SPECIAL: &str = "src/a space\t'quote' $(touch injected)\nfile.txt";

fn log_contains(log: &mut File, value: &str) -> bool {
    log.seek(SeekFrom::Start(0)).unwrap();
    let mut content = String::new();
    log.read_to_string(&mut content).unwrap();
    content.contains(value)
}

fn result_count(log: &mut File) -> usize {
    log.seek(SeekFrom::Start(0)).unwrap();
    let mut content = String::new();
    log.read_to_string(&mut content).unwrap();
    content.matches("Funzzy results").count()
}

fn write_config(fixture: &Path, content: &str) {
    fs::write(fixture.join(".watch.yaml"), content).unwrap();
}

#[test]
fn shell_jobs_read_exact_json_batch_in_both_modes_from_different_cwds() {
    let config = r#"on:
  watch_backend: poll
  poll_interval: 20ms
execution:
  env:
    FZZ_TRIGGER_PATHS_FILE: stale-execution
jobs:
  - name: root
    run: 'cp "$FZZ_TRIGGER_PATHS_FILE" root.json; printf "%s" "$FZZ_TRIGGER_PATHS_FILE" > root-pointer'
    change: 'src/**'
    env:
      FZZ_TRIGGER_PATHS_FILE: stale-job
  - name: nested
    cwd: src
    run: 'cp "$FZZ_TRIGGER_PATHS_FILE" ../nested.json'
    change: 'src/**'
"#;
    for (mode, non_blocking) in [("blocking", false), ("nonblocking", true)] {
        setup::with_config(
            Path::new(".watch.yaml"),
            &format!("qa_batch_{mode}.log"),
            |cmd, mut log, fixture| {
                let trigger = fixture.join(SPECIAL);
                fs::write(&trigger, "before").unwrap();
                write_config(fixture, config);
                cmd.env(ENV, "stale-inherited");
                if non_blocking {
                    cmd.env("FUNZZY_NON_BLOCK", "true");
                }
                let mut child = cmd.stdin(Stdio::piped()).spawn().unwrap();
                defer!({
                    let _ = child.kill();
                    let _ = child.wait();
                });
                wait_until!(log_contains(&mut log, "Watching..."), "watcher not ready");
                fs::write(&trigger, "after").unwrap();
                wait_until!(
                    log_contains(&mut log, "Funzzy results"),
                    "generation not complete"
                );
                let expected = serde_json::json!([trigger.to_str().unwrap()]);
                let root: serde_json::Value = serde_json::from_slice(
                    &fs::read(fixture.join("root.json")).expect("root must read manifest"),
                )
                .unwrap();
                let nested: serde_json::Value = serde_json::from_slice(
                    &fs::read(fixture.join("nested.json")).expect("nested must read manifest"),
                )
                .unwrap();
                assert_eq!(root, expected);
                assert_eq!(nested, expected);
                let pointer = fs::read_to_string(fixture.join("root-pointer")).unwrap();
                assert!(Path::new(&pointer).is_absolute());
                assert!(
                    !Path::new(&pointer).starts_with(fixture),
                    "manifest must be outside workspace"
                );
                assert!(
                    !Path::new(&pointer).exists(),
                    "manifest must be cleaned after generation"
                );
                assert!(!fixture.join("injected").exists());
            },
        );
    }
}

#[test]
fn manifest_stays_outside_workspace_when_tmpdir_is_inside_it() {
    let config = r#"on:
  watch_backend: poll
  poll_interval: 20ms
jobs:
  - name: recorder
    run: 'printf "%s" "$FZZ_TRIGGER_PATHS_FILE" > pointer'
    change: 'src/**'
"#;
    setup::with_config(
        Path::new(".watch.yaml"),
        "qa_batch_workspace_tmp.log",
        |cmd, mut log, fixture| {
            let trigger = fixture.join("src/one.txt");
            fs::write(&trigger, "before").unwrap();
            let tmp = fixture.join("local-tmp");
            fs::create_dir(&tmp).unwrap();
            write_config(fixture, config);
            let mut child = cmd
                .env("TMPDIR", &tmp)
                .stdin(Stdio::piped())
                .spawn()
                .unwrap();
            defer!({
                let _ = child.kill();
                let _ = child.wait();
            });
            wait_until!(log_contains(&mut log, "Watching..."), "watcher not ready");
            fs::write(trigger, "after").unwrap();
            wait_until!(result_count(&mut log) >= 1, "generation did not finish");
            let pointer = fs::read_to_string(fixture.join("pointer")).unwrap();
            assert!(
                !Path::new(&pointer).starts_with(fixture),
                "manifest created inside workspace: {pointer}"
            );
        },
    );
}

#[test]
fn direct_argv_exec_strips_init_pointer_and_reads_triggered_manifest() {
    setup::with_output("qa_batch_argv.log", |cmd, mut log, fixture| {
        let trigger = fixture.join("src/argv file.txt");
        fs::write(&trigger, "before").unwrap();
        let input = fixture.join("argv-input.txt");
        fs::write(&input, "src/argv file.txt\n").unwrap();
        let mut child = cmd
            .env(ENV, "stale-inherited")
            .args([
                "exec", "--", "sh", "-c",
                "if [ \"${FZZ_TRIGGER_PATHS_FILE+x}\" = x ]; then cp \"$FZZ_TRIGGER_PATHS_FILE\" argv.json; else touch argv-unset; fi",
            ])
            .stdin(Stdio::from(File::open(input).unwrap()))
            .spawn()
            .unwrap();
        defer!({
            let _ = child.kill();
            let _ = child.wait();
        });
        wait_until!(
            log_contains(&mut log, "Funzzy results"),
            "exec init did not complete"
        );
        assert!(
            fixture.join("argv-unset").exists(),
            "pathless exec inherited stale pointer"
        );
        fs::write(&trigger, "after").unwrap();
        wait_until!(
            result_count(&mut log) >= 2,
            "triggered argv generation did not finish"
        );
        let bytes = fs::read(fixture.join("argv.json")).unwrap();
        let actual: serde_json::Value = serde_json::from_slice(&bytes).unwrap();
        assert_eq!(actual, serde_json::json!([trigger.to_str().unwrap()]));
    });
}

#[test]
fn manual_pathless_job_strips_inherited_and_configured_manifest_pointer() {
    let config = r#"execution:
  env:
    FZZ_TRIGGER_PATHS_FILE: stale-execution
jobs:
  - name: manual
    trigger: manual
    run: 'if [ "${FZZ_TRIGGER_PATHS_FILE+x}" = x ]; then printf "%s" "$FZZ_TRIGGER_PATHS_FILE" > result; else printf unset > result; fi'
    env:
      FZZ_TRIGGER_PATHS_FILE: stale-job
"#;
    setup::with_config(
        Path::new(".watch.yaml"),
        "qa_batch_manual.log",
        |cmd, _, fixture| {
            write_config(fixture, config);
            let output = cmd
                .env(ENV, "stale-inherited")
                .args(["run", "manual"])
                .output()
                .unwrap();
            assert!(output.status.success());
            assert_eq!(fs::read_to_string(fixture.join("result")).unwrap(), "unset");
        },
    );
}

#[test]
fn pathless_init_and_ctrl_g_strip_stale_pointer_in_both_modes() {
    for (mode, non_blocking) in [("blocking", false), ("nonblocking", true)] {
        for (kind, init) in [("init", true), ("ctrl_g", false)] {
            let config = format!(
                r#"on:
  watch_backend: poll
  poll_interval: 20ms
execution:
  env:
    FZZ_TRIGGER_PATHS_FILE: stale-execution
jobs:
  - name: recorder
    run: 'if [ "${{FZZ_TRIGGER_PATHS_FILE+x}}" = x ]; then printf "%s" "$FZZ_TRIGGER_PATHS_FILE" > result; else printf unset > result; fi'
    change: 'src/**'
    run_on_init: {init}
    env:
      FZZ_TRIGGER_PATHS_FILE: stale-job
"#
            );
            setup::with_config(
                Path::new(".watch.yaml"),
                &format!("qa_batch_{kind}_{mode}.log"),
                |cmd, mut log, fixture| {
                    write_config(fixture, &config);
                    cmd.env(ENV, "stale-inherited");
                    if non_blocking {
                        cmd.env("FUNZZY_NON_BLOCK", "true");
                    }
                    let mut child = cmd.stdin(Stdio::piped()).spawn().unwrap();
                    let mut input = child.stdin.take().unwrap();
                    defer!({
                        let _ = child.kill();
                        let _ = child.wait();
                    });
                    if !init {
                        wait_until!(log_contains(&mut log, "Watching..."), "watcher not ready");
                        input.write_all(&[0x07]).unwrap();
                    }
                    wait_until!(
                        result_count(&mut log) >= 1,
                        "pathless generation did not finish"
                    );
                    assert_eq!(fs::read_to_string(fixture.join("result")).unwrap(), "unset");
                },
            );
        }
    }
}

#[test]
fn retry_reads_original_batch_in_both_watch_modes() {
    let config = r#"on:
  socket: control.sock
  watch_backend: poll
  poll_interval: 20ms
jobs:
  - name: flaky
    run: 'if test -f allow; then cp "$FZZ_TRIGGER_PATHS_FILE" retry.json; else cp "$FZZ_TRIGGER_PATHS_FILE" source.json; exit 1; fi'
    change: 'src/**'
"#;
    for (mode, non_blocking) in [("blocking", false), ("nonblocking", true)] {
        setup::with_config(
            Path::new(".watch.yaml"),
            &format!("qa_batch_retry_{mode}.log"),
            |cmd, mut log, fixture| {
                let trigger = fixture.join("src/retry.txt");
                fs::write(&trigger, "before").unwrap();
                write_config(fixture, config);
                if non_blocking {
                    cmd.env("FUNZZY_NON_BLOCK", "true");
                }
                let mut child = cmd.stdin(Stdio::piped()).spawn().unwrap();
                defer!({
                    let _ = child.kill();
                    let _ = child.wait();
                });
                wait_until!(
                    fixture.join("control.sock").exists(),
                    "control socket not ready"
                );
                fs::write(&trigger, "after").unwrap();
                wait_until!(
                    log_contains(&mut log, "has failed with exit status: 1"),
                    "source generation did not fail"
                );
                let source: serde_json::Value = serde_json::from_slice(
                    &fs::read(fixture.join("source.json")).expect("source manifest missing"),
                )
                .unwrap();
                assert_eq!(source, serde_json::json!([trigger.to_str().unwrap()]));
                fs::write(fixture.join("allow"), "pass").unwrap();
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
                    .unwrap();
                assert!(
                    retry.status.success(),
                    "retry failed: {}",
                    String::from_utf8_lossy(&retry.stderr)
                );
                let retried: serde_json::Value = serde_json::from_slice(
                    &fs::read(fixture.join("retry.json")).expect("retry manifest missing"),
                )
                .unwrap();
                assert_eq!(retried, source);
            },
        );
    }
}

#[test]
fn legacy_paths_expands_to_changed_path_in_both_watch_modes() {
    let config = r#"on:
  watch_backend: poll
  poll_interval: 20ms
jobs:
  - name: legacy
    run: 'printf "%s" {{paths}} > legacy-result'
    change: 'src/**'
"#;
    for (mode, non_blocking) in [("blocking", false), ("nonblocking", true)] {
        setup::with_config(
            Path::new(".watch.yaml"),
            &format!("qa_legacy_batch_{mode}.log"),
            |cmd, mut log, fixture| {
                let trigger = fixture.join("src/legacy name.txt");
                fs::write(&trigger, "before").unwrap();
                write_config(fixture, config);
                if non_blocking {
                    cmd.env("FUNZZY_NON_BLOCK", "true");
                }
                let mut child = cmd.stdin(Stdio::piped()).spawn().unwrap();
                defer!({
                    let _ = child.kill();
                    let _ = child.wait();
                });
                wait_until!(log_contains(&mut log, "Watching..."), "watcher not ready");
                fs::write(&trigger, "after").unwrap();
                wait_until!(
                    log_contains(&mut log, "Funzzy results"),
                    "generation not complete"
                );
                assert_eq!(
                    fs::read_to_string(fixture.join("legacy-result")).unwrap(),
                    trigger.to_str().unwrap()
                );
            },
        );
    }
}
