//! Black-box tests for retrying retained terminal generations exactly.
//!
//! These cover the CLI/control-socket contract, including eligibility,
//! idempotency, and the guarantee that a rejected retry schedules no work.

#![cfg(all(feature = "test-integration", unix))]

#[path = "common/lib.rs"]
mod common;

use nix::sys::signal::{kill, Signal};
use nix::unistd::Pid;
use serde_json::{Deserializer, Value};
use std::fs::{self, File};
use std::hash::{Hash, Hasher};
use std::io::{BufRead, BufReader, Write};
use std::os::unix::net::UnixStream;
use std::path::Path;
use std::process::{Child, Command, Output, Stdio};
use std::time::Duration;

const SOURCE_UNAVAILABLE: i64 = -32030;
const SOURCE_NOT_TERMINAL: i64 = -32031;
const STALE_REVISION: i64 = -32032;
const NO_RETRYABLE_JOBS: i64 = -32033;
const UNSUPPORTED_SERVICE: i64 = -32034;
const WATCHER_BUSY: i64 = -32035;

struct Watcher {
    child: Child,
}

impl Drop for Watcher {
    fn drop(&mut self) {
        let _ = kill(Pid::from_raw(self.child.id() as i32), Signal::SIGTERM);
        for _ in 0..50 {
            if self.child.try_wait().ok().flatten().is_some() {
                break;
            }
            std::thread::sleep(Duration::from_millis(100));
        }
        let _ = self.child.kill();
        let _ = self.child.wait();
    }
}

fn write_config(directory: &Path, config: &str) {
    fs::write(directory.join(".watch.yaml"), config).expect("write watcher config");
}

fn start_watcher(command: &mut Command, directory: &Path, bail: bool) -> Watcher {
    let child_log = File::create(directory.join("child.log")).expect("create watcher log");
    command
        .env_remove("FUNZZY_BAIL")
        .env_remove("FUNZZY_NON_BLOCK")
        .env("_TEST_FUNZZY_BAIL", if bail { "1" } else { "0" })
        .env("_TEST_FUNZZY_NON_BLOCK", "0")
        .env("_TEST_FUNZZY_COLORED", "0")
        .arg("watch")
        .stdin(Stdio::null())
        .stdout(Stdio::from(child_log.try_clone().expect("clone log")))
        .stderr(Stdio::from(child_log));
    Watcher {
        child: command.spawn().expect("start watcher"),
    }
}

fn with_watcher(test_name: &str, config: &str, bail: bool, test: impl FnOnce(&Path, Watcher)) {
    let mut hasher = std::collections::hash_map::DefaultHasher::new();
    test_name.hash(&mut hasher);
    let output_file = format!("r{:06x}", hasher.finish() & 0x00ff_ffff);
    let log_path = std::env::current_dir()
        .expect("current directory")
        .join(format!("{output_file}-{}", std::process::id()));
    defer!({
        let _ = fs::remove_file(&log_path);
    });
    let config_path = std::fs::canonicalize(std::env::temp_dir())
        .expect("canonicalize temporary directory")
        .join(format!(
            "funzzy-fixture-{}-{output_file}",
            std::process::id()
        ))
        .join(".watch.yaml");
    common::with_config(&config_path, &output_file, |command, _log, directory| {
        write_config(directory, config);
        let watcher = start_watcher(command, directory, bail);
        wait_until_socket(directory);
        test(directory, watcher);
    });
}

fn wait_until_socket(directory: &Path) {
    let socket = directory.join("sock");
    wait_until!(
        socket.exists(),
        "control socket was not created at {}; child log: {}",
        socket.display(),
        fs::read_to_string(directory.join("child.log")).unwrap_or_default()
    );
}

fn run_binary(directory: &Path, binary: &str, args: &[&str]) -> Output {
    Command::new(binary)
        .current_dir(directory)
        .env_remove("FUNZZY_BAIL")
        .env_remove("FUNZZY_NON_BLOCK")
        .env("_TEST_FUNZZY_COLORED", "0")
        .args(args)
        .output()
        .expect("run control client")
}

fn run_cli(directory: &Path, args: &[&str]) -> Output {
    run_binary(directory, env!("CARGO_BIN_EXE_fzz"), args)
}

fn run_retry(directory: &Path, generation: u64, wait_timeout: Option<&str>) -> Output {
    let mut command = Command::new(env!("CARGO_BIN_EXE_fzz"));
    command
        .current_dir(directory)
        .env_remove("FUNZZY_BAIL")
        .env_remove("FUNZZY_NON_BLOCK")
        .env("_TEST_FUNZZY_COLORED", "0")
        .args(["control", "--format", "json", "retry", "--generation"])
        .arg(generation.to_string());
    if let Some(timeout) = wait_timeout {
        command.args(["--wait", "--timeout", timeout]);
    }
    command.output().expect("run exact-generation retry")
}

fn json_documents(output: &Output) -> Vec<Value> {
    let stdout = String::from_utf8_lossy(&output.stdout);
    let payload = stdout
        .split_once("Error:")
        .map_or(stdout.as_ref(), |(json, _)| json)
        .trim();
    Deserializer::from_str(payload)
        .into_iter::<Value>()
        .map(|document| {
            document.unwrap_or_else(|error| panic!("invalid JSON response ({error}): {stdout}"))
        })
        .collect()
}

fn result_document(output: &Output) -> Value {
    json_documents(output)
        .into_iter()
        .next()
        .unwrap_or_else(|| {
            panic!(
                "JSON document missing; stdout={} stderr={}",
                String::from_utf8_lossy(&output.stdout),
                String::from_utf8_lossy(&output.stderr)
            )
        })
}

fn document_body(document: &Value) -> &Value {
    document.get("result").unwrap_or(document)
}

fn assert_task_outputs(output: &Output, expected: &[(&str, &str)]) {
    assert!(output.status.success(), "output: {output:?}");
    let document = result_document(output);
    let tasks = document_body(&document)["tasks"]
        .as_array()
        .expect("output task list");
    assert_eq!(tasks.len(), expected.len(), "output: {document}");

    for (task, (expected_id, expected_content)) in tasks.iter().zip(expected) {
        assert_eq!(task["id"], *expected_id, "task attribution: {document}");
        assert!(
            task["stdout"]["content"]
                .as_str()
                .is_some_and(|content| content.contains(expected_content)),
            "task output content: {document}"
        );
    }
}

fn assert_error_code(output: &Output, expected: i64) {
    assert_eq!(output.status.code(), Some(1), "retry error: {output:?}");
    let document = result_document(output);
    assert_eq!(document["error"]["code"], expected, "error: {document}");
}

fn emit(directory: &Path, path: &str) -> u64 {
    let output = run_cli(directory, &["control", "emit", path]);
    assert!(
        output.status.success(),
        "emit {path}: {}",
        String::from_utf8_lossy(&output.stdout)
    );
    String::from_utf8_lossy(&output.stdout)
        .lines()
        .find_map(|line| line.strip_prefix("scheduled generation: "))
        .unwrap_or_else(|| panic!("scheduled generation missing: {output:?}"))
        .parse()
        .expect("numeric generation")
}

fn await_generation(directory: &Path, generation: u64, timeout: &str) -> (Output, Value) {
    let generation = generation.to_string();
    let output = run_cli(
        directory,
        &[
            "control",
            "--format",
            "json",
            "await",
            "--generation",
            &generation,
            "--timeout",
            timeout,
        ],
    );
    let document = result_document(&output);
    let observation = document.get("result").cloned().unwrap_or(document);
    (output, serde_json::json!({"result": observation}))
}

fn status(directory: &Path) -> Option<Value> {
    let socket = UnixStream::connect(directory.join("sock")).ok()?;
    let mut socket = socket;
    writeln!(
        socket,
        r#"{{"jsonrpc":"2.0","id":"status","method":"status"}}"#
    )
    .ok()?;
    let mut line = String::new();
    BufReader::new(socket).read_line(&mut line).ok()?;
    serde_json::from_str::<Value>(&line)
        .ok()?
        .get("result")
        .cloned()
}

fn wait_for_log(directory: &Path, needle: &str) {
    wait_until!(
        fs::read_to_string(directory.join("child.log")).is_ok_and(|log| log.contains(needle)),
        "watcher log did not contain {needle:?}"
    );
}

fn wait_for_state(directory: &Path, generation: u64, expected: &str) {
    wait_until!(
        status(directory).is_some_and(|snapshot| {
            snapshot["generation"].as_u64() == Some(generation)
                && snapshot["state"].as_str() == Some(expected)
        }),
        "generation {generation} did not reach {expected}"
    );
}

fn retry_result(output: &Output) -> Value {
    assert!(
        output.status.success(),
        "retry failed: stdout={} stderr={}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    );
    let document = result_document(output);
    document.get("result").cloned().unwrap_or(document)
}

fn retry_run_id(result: &Value) -> u64 {
    result["runId"]
        .as_u64()
        .unwrap_or_else(|| panic!("retry response has no runId: {result}"))
}

fn retry_error(directory: &Path, generation: u64, expected: i64) -> Output {
    let output = run_retry(directory, generation, None);
    assert_error_code(&output, expected);
    output
}

#[test]
fn retry_replays_only_failed_jobs_in_one_generation_and_is_idempotent() {
    let config = r#"
on:
  socket: sock
  watch_backend: poll
  poll_interval: 50ms
jobs:
  - name: failed-one
    change: trigger.txt
    run: 'printf "failed-one-output\n"; printf "run\n" >> failed-one.log; if test -f retry-ok; then exit 0; else exit 1; fi'
  - name: failed-two
    change: trigger.txt
    run: 'printf "failed-two-output\n"; printf "run\n" >> failed-two.log; if test -f retry-ok; then exit 0; else exit 1; fi'
  - name: passed
    change: trigger.txt
    run: 'printf "passed-output\n"; printf "run\n" >> passed.log'
"#;
    with_watcher("failed-only", config, false, |directory, _watcher| {
        let source_generation = emit(&directory, "trigger.txt");
        let (source_output, source) = await_generation(&directory, source_generation, "30s");
        assert_eq!(source["result"]["terminalReason"], "failed");
        assert_eq!(source["result"]["snapshot"]["state"], "failed");
        assert_eq!(source_output.status.code(), Some(1));
        assert_eq!(
            fs::read_to_string(directory.join("failed-one.log")).unwrap(),
            "run\n"
        );
        assert_eq!(
            fs::read_to_string(directory.join("failed-two.log")).unwrap(),
            "run\n"
        );
        assert_eq!(
            fs::read_to_string(directory.join("passed.log")).unwrap(),
            "run\n"
        );

        fs::write(directory.join("retry-ok"), "allow").unwrap();
        let retry_output = run_retry(&directory, source_generation, None);
        let retry = retry_result(&retry_output);
        let retry_generation = retry_run_id(&retry);
        assert_eq!(retry["sourceGeneration"].as_u64(), Some(source_generation));
        assert_ne!(retry_generation, source_generation);
        assert_eq!(retry_generation, source_generation + 1);

        let (retry_output, terminal) = await_generation(&directory, retry_generation, "30s");
        assert!(retry_output.status.success());
        assert_eq!(terminal["result"]["terminalReason"], "passed");
        assert_eq!(terminal["result"]["snapshot"]["state"], "passed");
        assert_eq!(
            fs::read_to_string(directory.join("failed-one.log")).unwrap(),
            "run\nrun\n"
        );
        assert_eq!(
            fs::read_to_string(directory.join("failed-two.log")).unwrap(),
            "run\nrun\n"
        );
        assert_eq!(
            fs::read_to_string(directory.join("passed.log")).unwrap(),
            "run\n"
        );

        let retry_id = retry_generation.to_string();
        let retry_json_output = run_cli(
            directory,
            &[
                "control",
                "--format",
                "json",
                "output",
                "--generation",
                &retry_id,
                "--full",
            ],
        );
        assert_task_outputs(
            &retry_json_output,
            &[
                ("failed-one", "failed-one-output"),
                ("failed-two", "failed-two-output"),
            ],
        );
        let output = run_cli(
            directory,
            &["control", "output", "--generation", &retry_id, "--full"],
        );
        assert!(output.status.success(), "retry output: {output:?}");
        let output = String::from_utf8_lossy(&output.stdout);
        assert!(output.contains("failed-one-output"), "output: {output}");
        assert!(output.contains("failed-two-output"), "output: {output}");
        assert!(!output.contains("passed-output"), "output: {output}");

        let source_id = source_generation.to_string();
        let source_json_output = run_cli(
            directory,
            &[
                "control",
                "--format",
                "json",
                "output",
                "--generation",
                &source_id,
                "--full",
            ],
        );
        assert_task_outputs(
            &source_json_output,
            &[
                ("failed-one", "failed-one-output"),
                ("failed-two", "failed-two-output"),
                ("passed", "passed-output"),
            ],
        );
        let source_output = run_cli(
            directory,
            &["control", "output", "--generation", &source_id, "--full"],
        );
        assert!(
            source_output.status.success(),
            "source output: {source_output:?}"
        );
        let source_output = String::from_utf8_lossy(&source_output.stdout);
        assert!(
            source_output.contains("failed-one-output"),
            "source output: {source_output}"
        );
        assert!(
            source_output.contains("failed-two-output"),
            "source output: {source_output}"
        );
        assert!(
            source_output.contains("passed-output"),
            "source output: {source_output}"
        );

        let (_, source_again) = await_generation(&directory, source_generation, "1s");
        assert_eq!(source_again["result"]["terminalReason"], "failed");

        let duplicate = retry_result(&run_retry(&directory, source_generation, None));
        assert_eq!(retry_run_id(&duplicate), retry_generation);
        assert_eq!(
            status(directory).unwrap()["generation"].as_u64(),
            Some(retry_generation)
        );
    });
}

fn assert_filepath_survives_chained_retries(
    directory: &Path,
    path: &str,
    allow_file: &str,
    log_file: &str,
) {
    let source_generation = emit(directory, path);
    let (source_output, _) = await_generation(directory, source_generation, "30s");
    assert_eq!(source_output.status.code(), Some(1));

    let first_retry = retry_result(&run_retry(directory, source_generation, None));
    let first_retry_generation = retry_run_id(&first_retry);
    let (first_output, _) = await_generation(directory, first_retry_generation, "30s");
    assert_eq!(first_output.status.code(), Some(1));

    fs::write(directory.join(allow_file), "pass").unwrap();
    let second_retry = retry_result(&run_retry(directory, first_retry_generation, None));
    let second_retry_generation = retry_run_id(&second_retry);
    let (second_output, _) = await_generation(directory, second_retry_generation, "30s");
    assert!(second_output.status.success());
    assert_eq!(
        fs::read_to_string(directory.join(log_file)).unwrap(),
        format!("{path}\n{path}\n{path}\n")
    );
}

#[test]
fn retries_preserve_original_filepath_context_across_failed_retries() {
    let config = r#"
on:
  socket: sock
  watch_backend: poll
jobs:
  - name: retry-path
    change: 'retry:trigger.txt'
    run: 'echo "{{filepath}}" >> retry-paths.log; if test -f retry-allow; then exit 0; else exit 1; fi'
  - name: control-path
    change: 'control:trigger.txt'
    run: 'echo "{{filepath}}" >> control-paths.log; if test -f control-allow; then exit 0; else exit 1; fi'
"#;
    with_watcher("filepath", config, false, |directory, watcher| {
        assert_filepath_survives_chained_retries(
            &directory,
            "retry:trigger.txt",
            "retry-allow",
            "retry-paths.log",
        );
        assert_filepath_survives_chained_retries(
            &directory,
            "control:trigger.txt",
            "control-allow",
            "control-paths.log",
        );
        drop(watcher);
    });
}

#[test]
fn retry_preserves_arbitrary_colon_prefixed_filepaths() {
    let config = r#"
on:
  socket: sock
  watch_backend: poll
jobs:
  - name: arbitrary-prefixed
    change: 'workspace:trigger.txt'
    run: 'echo "{{filepath}}" >> workspace-paths.log; if test -f allow-workspace; then exit 0; else exit 1; fi'
"#;
    with_watcher("colon-filepath", config, false, |directory, _watcher| {
        let source_generation = emit(directory, "workspace:trigger.txt");
        let (source_output, _) = await_generation(directory, source_generation, "30s");
        assert_eq!(source_output.status.code(), Some(1));
        fs::write(directory.join("allow-workspace"), "pass").unwrap();

        let retry = retry_result(&run_retry(directory, source_generation, None));
        let retry_generation = retry_run_id(&retry);
        let (retry_output, terminal) = await_generation(directory, retry_generation, "30s");
        assert!(retry_output.status.success());
        assert_eq!(terminal["result"]["terminalReason"], "passed");
        assert_eq!(
            fs::read_to_string(directory.join("workspace-paths.log"))
                .unwrap()
                .lines()
                .collect::<Vec<_>>(),
            ["workspace:trigger.txt", "workspace:trigger.txt"]
        );
    });
}

#[test]
fn both_binaries_advertise_retry_and_render_all_supported_formats() {
    for (binary, args, expected) in [
        (
            env!("CARGO_BIN_EXE_fzz"),
            vec!["control", "retry", "--help"],
            vec![
                "--generation",
                "--wait",
                "--timeout",
                "fzz control --format toon retry --generation N",
            ],
        ),
        (
            env!("CARGO_BIN_EXE_funzzy"),
            vec!["ctl", "retry", "--help"],
            vec![
                "--generation",
                "--wait",
                "--timeout",
                "fzz control --format toon retry --generation N",
            ],
        ),
        (
            env!("CARGO_BIN_EXE_fzz"),
            vec!["--help"],
            vec![
                "fzz control --format toon status",
                "fzz control --format toon retry",
                "fzz control --format toon await",
                "fzz control --format toon output",
            ],
        ),
    ] {
        let output = Command::new(binary)
            .args(&args)
            .output()
            .expect("run retry help");
        assert!(output.status.success(), "help: {output:?}");
        let help = String::from_utf8_lossy(&output.stdout);
        for expected in expected {
            assert!(help.contains(expected), "missing {expected:?}: {help}");
        }
    }

    let config = r#"
on:
  socket: sock
  watch_backend: poll
jobs:
  - name: failed
    change: format.txt
    run: 'exit 1'
"#;
    with_watcher("formats", config, false, |directory, watcher| {
        let source_generation = emit(&directory, "format.txt");
        let (source_output, _) = await_generation(&directory, source_generation, "30s");
        assert_eq!(source_output.status.code(), Some(1));
        let source = source_generation.to_string();

        let human = run_cli(
            &directory,
            &[
                "control",
                "--format",
                "human",
                "retry",
                "--generation",
                &source,
            ],
        );
        assert!(human.status.success(), "human: {human:?}");
        let human = String::from_utf8_lossy(&human.stdout);
        assert!(human.contains(&format!("source generation: {source}")));
        let retry_generation = human
            .lines()
            .find_map(|line| line.strip_prefix("scheduled generation: "))
            .expect("human retry generation")
            .to_owned();
        assert!(human.contains("retried tasks: failed"));

        let json = run_binary(
            &directory,
            env!("CARGO_BIN_EXE_funzzy"),
            &["ctl", "--format", "json", "retry", "--generation", &source],
        );
        let json_result = retry_result(&json);
        assert_eq!(retry_run_id(&json_result).to_string(), retry_generation);
        assert_eq!(
            json_result["sourceGeneration"].as_u64(),
            Some(source_generation)
        );

        let toon = run_cli(
            &directory,
            &["ctl", "--format", "toon", "retry", "--generation", &source],
        );
        assert!(toon.status.success(), "TOON: {toon:?}");
        let toon = String::from_utf8_lossy(&toon.stdout);
        assert!(toon.contains(&retry_generation), "TOON response: {toon}");
        assert!(toon.contains("failed"), "TOON response: {toon}");

        let retry_generation_id = retry_generation.parse::<u64>().unwrap();
        let (retry_output, _) = await_generation(&directory, retry_generation_id, "30s");
        assert_eq!(retry_output.status.code(), Some(1));

        let toon_status = run_cli(&directory, &["control", "--format", "toon", "status"]);
        assert!(toon_status.status.success(), "TOON status: {toon_status:?}");
        assert!(String::from_utf8_lossy(&toon_status.stdout).contains(&retry_generation));

        let toon_await = run_cli(
            &directory,
            &[
                "control",
                "--format",
                "toon",
                "await",
                "--generation",
                &retry_generation,
                "--timeout",
                "30s",
            ],
        );
        assert_eq!(
            toon_await.status.code(),
            Some(1),
            "TOON await: {toon_await:?}"
        );
        assert!(String::from_utf8_lossy(&toon_await.stdout).contains("failed"));

        let toon_output = run_cli(
            &directory,
            &[
                "control",
                "--format",
                "toon",
                "output",
                "--generation",
                &retry_generation,
                "--tail",
                "80",
            ],
        );
        assert!(toon_output.status.success(), "TOON output: {toon_output:?}");
        assert!(String::from_utf8_lossy(&toon_output.stdout).contains(&retry_generation));

        let capabilities = run_cli(&directory, &["control", "--format", "json", "capabilities"]);
        assert!(
            capabilities.status.success(),
            "capabilities: {capabilities:?}"
        );
        let capabilities_document = result_document(&capabilities);
        let methods = document_body(&capabilities_document)["methods"]
            .as_array()
            .expect("capability methods");
        for method in ["status", "targets", "run", "retry"] {
            assert!(
                methods.iter().any(|value| value == method),
                "methods: {methods:?}"
            );
        }
        drop(watcher);
    });
}

#[test]
fn retry_accepts_a_timedout_task() {
    let config = r#"
on:
  socket: sock
  watch_backend: poll
  poll_interval: 50ms
jobs:
  - name: slow
    change: timed.txt
    timeout: 2s
    run: 'printf "run\n" >> attempts.log; if test -f retry-ok; then exit 0; else sleep 30; fi'
"#;
    with_watcher("timedout", config, false, |directory, watcher| {
        let source_generation = emit(&directory, "timed.txt");
        let (source_output, source) = await_generation(&directory, source_generation, "30s");
        assert_eq!(source_output.status.code(), Some(1));
        assert_eq!(
            source["result"]["snapshot"]["tasks"][0]["state"],
            "timedout"
        );

        fs::write(directory.join("retry-ok"), "allow").unwrap();
        let retry = retry_result(&run_retry(&directory, source_generation, None));
        let retry_generation = retry_run_id(&retry);
        let (retry_output, terminal) = await_generation(&directory, retry_generation, "30s");
        assert!(retry_output.status.success());
        assert_eq!(terminal["result"]["terminalReason"], "passed");
        assert_eq!(
            fs::read_to_string(directory.join("attempts.log")).unwrap(),
            "run\nrun\n"
        );
        drop(watcher);
    });
}

#[test]
fn retry_wait_timeout_reports_the_same_generation_without_resubmitting() {
    let config = r#"
on:
  socket: sock
  watch_backend: poll
  poll_interval: 50ms
jobs:
  - name: retry-slow
    change: retry.txt
    run: 'printf "run\n" >> attempts.log; if test -f retry-slow; then touch retry-started; while [ ! -f retry-release ]; do sleep 0.02; done; else exit 1; fi'
"#;
    with_watcher("wait-timeout", config, false, |directory, watcher| {
        let source_generation = emit(&directory, "retry.txt");
        let (source_output, source) = await_generation(&directory, source_generation, "30s");
        assert_eq!(source_output.status.code(), Some(1));
        assert_eq!(source["result"]["terminalReason"], "failed");
        fs::write(directory.join("retry-slow"), "slow pass").unwrap();

        let timed_out = run_retry(&directory, source_generation, Some("1s"));
        assert_eq!(timed_out.status.code(), Some(1));
        let documents = json_documents(&timed_out);
        let retry = documents
            .iter()
            .find(|document| document_body(document)["runId"].as_u64().is_some())
            .expect("retry schedule response");
        let retry_generation = document_body(retry)["runId"].as_u64().unwrap();
        let timeout = documents
            .iter()
            .find(|document| document_body(document)["terminalReason"] == "timeout")
            .expect("bounded await timeout observation");
        assert_eq!(
            document_body(timeout)["snapshot"]["generation"].as_u64(),
            Some(retry_generation)
        );
        wait_until!(
            directory.join("retry-started").exists(),
            "retry task did not start before its await timed out"
        );
        fs::write(directory.join("retry-release"), "release").unwrap();

        let (terminal_output, terminal) = await_generation(&directory, retry_generation, "10s");
        assert!(terminal_output.status.success());
        assert_eq!(terminal["result"]["terminalReason"], "passed");
        let duplicate = retry_result(&run_retry(&directory, source_generation, None));
        assert_eq!(retry_run_id(&duplicate), retry_generation);
        assert_eq!(
            fs::read_to_string(directory.join("attempts.log")).unwrap(),
            "run\nrun\n"
        );
        drop(watcher);
    });
}

#[test]
fn retry_rejects_a_source_with_no_failed_or_timedout_jobs() {
    let config = r#"
on:
  socket: sock
  watch_backend: poll
jobs:
  - name: passed
    change: passed.txt
    run: 'printf "run\n" >> passed.log'
"#;
    with_watcher("no-failures", config, false, |directory, watcher| {
        let source_generation = emit(&directory, "passed.txt");
        let (output, source) = await_generation(&directory, source_generation, "30s");
        assert!(output.status.success());
        assert_eq!(source["result"]["terminalReason"], "passed");

        let before = status(&directory).unwrap()["generation"].clone();
        retry_error(&directory, source_generation, NO_RETRYABLE_JOBS);
        assert_eq!(status(&directory).unwrap()["generation"], before);
        assert_eq!(
            fs::read_to_string(directory.join("passed.log")).unwrap(),
            "run\n"
        );
        drop(watcher);
    });
}

#[test]
fn retry_rejects_source_after_a_valid_config_reload() {
    let before = r#"
on:
  socket: sock
  watch_backend: poll
jobs:
  - name: failed
    change: trigger.txt
    run: 'echo run >> attempts.log; exit 1'
"#;
    let after = r#"
on:
  socket: sock
  watch_backend: poll
jobs:
  - name: failed
    change: trigger.txt
    run: 'echo run >> attempts.log; exit 1'
  - name: added-after-reload
    change: extra.txt
    run: 'true'
"#;
    with_watcher("reload-stale", before, false, |directory, watcher| {
        let source_generation = emit(&directory, "trigger.txt");
        let (source_output, _) = await_generation(&directory, source_generation, "30s");
        assert_eq!(source_output.status.code(), Some(1));

        write_config(directory, after);
        wait_for_log(directory, "hot-reloading to revision 2");

        retry_error(directory, source_generation, STALE_REVISION);
        assert_eq!(
            status(directory).unwrap()["generation"].as_u64(),
            Some(source_generation)
        );
        assert_eq!(
            fs::read_to_string(directory.join("attempts.log")).unwrap(),
            "run\n"
        );
        drop(watcher);
    });
}

#[test]
fn retry_rejects_a_generation_that_is_no_longer_the_latest_terminal_source() {
    let config = r#"
on:
  socket: sock
  watch_backend: poll
jobs:
  - name: failed
    change: failed.txt
    run: 'exit 1'
  - name: passed
    change: passed.txt
    run: 'true'
"#;
    with_watcher("stale-source", config, false, |directory, watcher| {
        let stale_generation = emit(&directory, "failed.txt");
        let (failed_output, _) = await_generation(&directory, stale_generation, "30s");
        assert_eq!(failed_output.status.code(), Some(1));
        let latest_generation = emit(&directory, "passed.txt");
        let (passed_output, _) = await_generation(&directory, latest_generation, "30s");
        assert!(passed_output.status.success());

        retry_error(&directory, stale_generation, SOURCE_UNAVAILABLE);
        assert_eq!(
            status(&directory).unwrap()["generation"].as_u64(),
            Some(latest_generation)
        );
        drop(watcher);
    });
}

#[test]
fn retry_rejects_running_and_cancelled_sources_as_nonterminal() {
    let config = r#"
on:
  socket: sock
  watch_backend: poll
jobs:
  - name: long-running
    change: running.txt
    run: 'sleep 30'
"#;
    with_watcher("nonterminal", config, false, |directory, watcher| {
        let source_generation = emit(&directory, "running.txt");
        wait_for_state(&directory, source_generation, "running");
        retry_error(&directory, source_generation, SOURCE_NOT_TERMINAL);

        let generation = source_generation.to_string();
        let cancelled = run_cli(
            &directory,
            &["control", "cancel", "--generation", &generation],
        );
        assert!(cancelled.status.success(), "cancel: {cancelled:?}");
        wait_for_state(&directory, source_generation, "cancelled");
        retry_error(&directory, source_generation, SOURCE_NOT_TERMINAL);
        assert_eq!(
            status(&directory).unwrap()["generation"].as_u64(),
            Some(source_generation)
        );
        drop(watcher);
    });
}

#[test]
fn retry_does_not_replay_skipped_jobs_from_a_fail_fast_generation() {
    let config = r#"
on:
  socket: sock
  watch_backend: poll
jobs:
  - name: failing
    change: trigger.txt
    run: 'printf "run\n" >> failed.log; if test -f retry-ok; then exit 0; else exit 1; fi'
  - name: skipped
    change: trigger.txt
    run: 'printf "run\n" >> skipped.log'
"#;
    with_watcher("skipped", config, true, |directory, watcher| {
        let source_generation = emit(&directory, "trigger.txt");
        let (source_output, source) = await_generation(&directory, source_generation, "30s");
        assert_eq!(source_output.status.code(), Some(1));
        assert_eq!(source["result"]["snapshot"]["tasks"][0]["state"], "failed");
        assert_eq!(
            source["result"]["snapshot"]["tasks"][1]["state"],
            "cancelled"
        );
        assert!(!directory.join("skipped.log").exists());

        fs::write(directory.join("retry-ok"), "allow").unwrap();
        let retry = retry_result(&run_retry(&directory, source_generation, None));
        let retry_generation = retry_run_id(&retry);
        let (retry_output, terminal) = await_generation(&directory, retry_generation, "30s");
        assert!(retry_output.status.success());
        assert_eq!(terminal["result"]["terminalReason"], "passed");
        assert_eq!(
            fs::read_to_string(directory.join("failed.log")).unwrap(),
            "run\nrun\n"
        );
        assert!(!directory.join("skipped.log").exists());
        drop(watcher);
    });
}

#[test]
fn retry_rejects_failed_service_without_scheduling_partial_work() {
    let config = r#"
on:
  socket: sock
  watch_backend: poll
jobs:
  - name: finite-failure
    change: service.txt
    run: 'echo run >> finite.log; exit 1'
  - name: unsupported-service
    service: true
    change: service.txt
    run: 'exit 1'
"#;
    with_watcher("service", config, false, |directory, watcher| {
        let source_generation = emit(&directory, "service.txt");
        let (source_output, source) = await_generation(&directory, source_generation, "30s");
        assert_eq!(source_output.status.code(), Some(1));
        assert_eq!(
            source["result"]["snapshot"]["tasks"]
                .as_array()
                .unwrap()
                .len(),
            2
        );
        assert_eq!(
            fs::read_to_string(directory.join("finite.log")).unwrap(),
            "run\n"
        );
        let before = status(&directory).unwrap()["generation"].clone();

        let error = run_retry(&directory, source_generation, None);
        assert_error_code(&error, UNSUPPORTED_SERVICE);
        assert!(
            String::from_utf8_lossy(&error.stdout).contains("unsupported-service"),
            "service name missing from error: {error:?}"
        );
        assert_eq!(status(&directory).unwrap()["generation"], before);
        drop(watcher);
    });
}

#[test]
fn retry_rejects_a_terminal_source_while_another_generation_is_running() {
    let config = r#"
on:
  socket: sock
  watch_backend: poll
jobs:
  - name: source-failure
    change: failed.txt
    run: 'exit 1'
  - name: blocker
    change: blocker.txt
    run: 'sleep 30'
"#;
    with_watcher("busy-active", config, false, |directory, watcher| {
        let source_generation = emit(&directory, "failed.txt");
        let (source_output, _) = await_generation(&directory, source_generation, "30s");
        assert_eq!(source_output.status.code(), Some(1));
        let active_generation = emit(&directory, "blocker.txt");
        wait_for_state(&directory, active_generation, "running");

        retry_error(&directory, source_generation, WATCHER_BUSY);
        let active = status(&directory).unwrap();
        assert_eq!(active["generation"].as_u64(), Some(active_generation));
        assert_eq!(active["state"], "running");
        drop(watcher);
    });
}
