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

fn started_changed(events: &Path) -> Vec<String> {
    let content = fs::read_to_string(events).unwrap();
    let started = content
        .lines()
        .filter_map(|line| serde_json::from_str::<serde_json::Value>(line).ok())
        .rfind(|event| event["event"] == "started" && event["batch"].is_number())
        .unwrap_or_else(|| panic!("correlated started event missing: {content}"));
    started["changed"]
        .as_array()
        .unwrap_or_else(|| panic!("correlated changed batch missing: {started}"))
        .iter()
        .map(|path| path.as_str().unwrap().to_owned())
        .collect()
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
fn selected_job_reads_observed_multi_path_batch_in_both_modes() {
    let config = r#"on:
  watch_backend: poll
  poll_interval: 20ms
  debounce: 500ms
jobs:
  - name: src-recorder
    run: 'cp "$FZZ_TRIGGER_PATHS_FILE" observed.json; touch batch-started; while [ ! -e release-batch ]; do sleep 0.05; done'
    change: 'src/**'
  - name: zzz-selector
    run: 'true'
    change: 'zzz/**'
"#;
    for (mode, non_blocking) in [("blocking", false), ("nonblocking", true)] {
        setup::with_config(
            Path::new(".watch.yaml"),
            &format!("qa_batch_observed_{mode}.log"),
            |cmd, mut log, fixture| {
                let src = fixture.join("src/observed.txt");
                let disjoint = fixture.join("zzz/observed.txt");
                let events = fixture.join("events.ndjson");
                fs::create_dir_all(fixture.join("zzz")).unwrap();
                fs::write(&src, "before").unwrap();
                fs::write(&disjoint, "before").unwrap();
                write_config(fixture, config);
                cmd.args(["--events", "events.ndjson"]);
                if non_blocking {
                    cmd.env("FUNZZY_NON_BLOCK", "true");
                }
                let mut child = cmd.stdin(Stdio::piped()).spawn().unwrap();
                defer!({
                    let _ = child.kill();
                    let _ = child.wait();
                });
                wait_until!(log_contains(&mut log, "Watching..."), "watcher not ready");

                fs::write(&disjoint, "after").unwrap();
                fs::write(&src, "after").unwrap();
                wait_until!(
                    fixture.join("batch-started").exists() && events.exists(),
                    "correlated generation did not start"
                );

                let observed = started_changed(&events);
                let decoded: Vec<String> =
                    serde_json::from_slice(&fs::read(fixture.join("observed.json")).unwrap())
                        .unwrap();

                assert_eq!(decoded, observed, "manifest must equal started event batch");
                assert!(
                    observed.contains(&src.display().to_string()),
                    "{observed:?}"
                );
                assert!(
                    observed.contains(&disjoint.display().to_string()),
                    "src-selected job lost disjoint zzz path: {observed:?}"
                );

                fs::write(fixture.join("release-batch"), "release").unwrap();
                wait_until!(result_count(&mut log) >= 1, "generation did not finish");
            },
        );
    }
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
fn replacing_and_cancelling_generations_cleans_their_distinct_manifests() {
    let config = r#"on:
  socket: control.sock
  watch_backend: poll
  poll_interval: 20ms
jobs:
  - name: slow
    run: 'printf "%s\n" "$FZZ_TRIGGER_PATHS_FILE" >> pointers; sleep 60'
    change: 'src/**'
"#;
    setup::with_config(
        Path::new(".watch.yaml"),
        "qa_batch_cancel.log",
        |cmd, mut log, fixture| {
            let trigger = fixture.join("src/cancel.txt");
            fs::write(&trigger, "before").unwrap();
            write_config(fixture, config);
            let mut child = cmd
                .env("FUNZZY_NON_BLOCK", "true")
                .stdin(Stdio::piped())
                .spawn()
                .unwrap();
            defer!({
                let _ = child.kill();
                let _ = child.wait();
            });
            wait_until!(log_contains(&mut log, "Watching..."), "watcher not ready");
            fs::write(&trigger, "first").unwrap();
            let pointers = fixture.join("pointers");
            wait_until!(
                fs::read_to_string(&pointers)
                    .map(|s| s.lines().count() == 1)
                    .unwrap_or(false),
                "first job did not start"
            );
            let first = fs::read_to_string(&pointers).unwrap().trim().to_owned();
            assert!(Path::new(&first).exists(), "active first manifest missing");
            fs::write(&trigger, "second change").unwrap();
            wait_until!(
                fs::read_to_string(&pointers)
                    .map(|s| s.lines().count() == 2)
                    .unwrap_or(false),
                "replacement job did not start"
            );
            let contents = fs::read_to_string(&pointers).unwrap();
            let second = contents.lines().nth(1).unwrap();
            assert_ne!(first, second);
            assert!(
                !Path::new(&first).exists(),
                "cancelled generation retained its manifest"
            );
            assert!(
                Path::new(second).exists(),
                "replacement manifest missing while active"
            );
            let status = Command::new(env!("CARGO_BIN_EXE_fzz"))
                .current_dir(fixture)
                .args(["control", "--format", "json", "status"])
                .output()
                .unwrap();
            assert!(status.status.success());
            let snapshot: serde_json::Value = serde_json::from_slice(&status.stdout).unwrap();
            let generation = snapshot["generation"]
                .as_u64()
                .expect("current generation")
                .to_string();
            let cancel = Command::new(env!("CARGO_BIN_EXE_fzz"))
                .current_dir(fixture)
                .args([
                    "control",
                    "--format",
                    "json",
                    "cancel",
                    "--generation",
                    &generation,
                    "--wait",
                    "--timeout",
                    "30s",
                ])
                .output()
                .unwrap();
            assert!(
                cancel.status.success(),
                "cancel failed: {}",
                String::from_utf8_lossy(&cancel.stderr)
            );
            wait_until!(
                !Path::new(second).exists(),
                "cancelled replacement retained its manifest"
            );
        },
    );
}

#[test]
fn ready_service_retains_original_manifest_while_later_generation_runs() {
    let config = r#"on:
  socket: control.sock
  watch_backend: poll
  poll_interval: 20ms
jobs:
  - name: service
    service: true
    run: 'printf "%s" "$FZZ_TRIGGER_PATHS_FILE" > service-pointer; printf "%s" "$$" > service-pid; exec sleep 60'
    change: 'src/service.txt'
    readiness:
      run: 'test -f service-pointer && cp "$FZZ_TRIGGER_PATHS_FILE" readiness.json'
      timeout: 5s
      interval: 20ms
  - name: later
    run: 'printf "%s" "$FZZ_TRIGGER_PATHS_FILE" > later-pointer; sleep 60'
    change: 'src/later.txt'
"#;
    setup::with_config(
        Path::new(".watch.yaml"),
        "qa_batch_service_overlap.log",
        |cmd, mut log, fixture| {
            let service_file = fixture.join("src/service.txt");
            let later_file = fixture.join("src/later.txt");
            fs::write(&service_file, "before").unwrap();
            fs::write(&later_file, "before").unwrap();
            write_config(fixture, config);
            let child = std::cell::RefCell::new(cmd.stdin(Stdio::piped()).spawn().unwrap());
            let child_id = child.borrow().id().to_string();
            defer!({
                let _ = child.borrow_mut().kill();
                let _ = child.borrow_mut().wait();
                if let Ok(pid) = fs::read_to_string(fixture.join("service-pid")) {
                    let _ = Command::new("kill")
                        .args(["-TERM", pid.trim()])
                        .stdout(Stdio::null())
                        .stderr(Stdio::null())
                        .status();
                }
            });
            wait_until!(log_contains(&mut log, "Watching..."), "watcher not ready");
            fs::write(&service_file, "service changed").unwrap();
            wait_until!(
                result_count(&mut log) >= 1,
                "service generation did not settle"
            );
            let service_pointer = fs::read_to_string(fixture.join("service-pointer")).unwrap();
            let service_manifest = Path::new(&service_pointer);
            assert!(service_manifest.exists(), "ready service lost its manifest");
            let readiness: serde_json::Value =
                serde_json::from_slice(&fs::read(fixture.join("readiness.json")).unwrap()).unwrap();
            assert_eq!(
                readiness,
                serde_json::json!([service_file.to_str().unwrap()])
            );
            fs::write(&later_file, "later changed").unwrap();
            let later_pointer_file = fixture.join("later-pointer");
            wait_until!(
                fs::read_to_string(&later_pointer_file)
                    .map(|s| !s.is_empty())
                    .unwrap_or(false),
                "later generation did not start"
            );
            let later_pointer = fs::read_to_string(later_pointer_file).unwrap();
            assert_ne!(service_pointer, later_pointer);
            let later_manifest = Path::new(&later_pointer);
            assert!(later_manifest.exists());
            let service: serde_json::Value =
                serde_json::from_slice(&fs::read(service_manifest).unwrap()).unwrap();
            let later: serde_json::Value =
                serde_json::from_slice(&fs::read(later_manifest).unwrap()).unwrap();
            assert_eq!(service, serde_json::json!([service_file.to_str().unwrap()]));
            assert_eq!(later, serde_json::json!([later_file.to_str().unwrap()]));
            let status = Command::new("kill")
                .args(["-TERM", &child_id])
                .status()
                .unwrap();
            assert!(status.success());
            wait_until!(
                !service_manifest.exists() && !later_manifest.exists(),
                "manifests leaked after watcher shutdown: service={} later={}",
                service_manifest.exists(),
                later_manifest.exists()
            );
            wait_until!(
                child.borrow_mut().try_wait().unwrap().is_some(),
                "watcher did not exit after SIGTERM"
            );
            let exit = child
                .borrow_mut()
                .try_wait()
                .unwrap()
                .expect("watcher exit status");
            assert_eq!(exit.code(), Some(143));
            assert!(
                !log_contains(&mut log, "restarted after an unexpected exit"),
                "service restarted during watcher shutdown"
            );
            let service_pid = fs::read_to_string(fixture.join("service-pid")).unwrap();
            let service_alive = Command::new("kill")
                .args(["-0", service_pid.trim()])
                .stdout(Stdio::null())
                .stderr(Stdio::null())
                .status()
                .unwrap()
                .success();
            assert!(!service_alive, "service process survived watcher shutdown");
        },
    );
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
