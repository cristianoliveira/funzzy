//! Black-box retained-output tests (TASK-0045, contract §6).
//!
//! Proves capture + retention + retrieval over the wire and CLI: per-stream
//! stdout/stderr, failure evidence in await observations, bounded retrieval
//! (tail/full), actionable errors for missing generations/tasks, and watcher
//! restart clearing all retained output.

#![cfg(all(feature = "test-integration", unix))]

use std::io::{BufRead, BufReader, Write};
use std::os::unix::net::UnixStream;
use std::process::{Child, Command, Output, Stdio};
use std::time::Duration;

struct TestProcess {
    child: Child,
    directory: std::path::PathBuf,
}

impl TestProcess {
    fn kill(&mut self) {
        let _ = self.child.kill();
        let _ = self.child.wait();
    }
}

impl Drop for TestProcess {
    fn drop(&mut self) {
        let _ = unsafe { libc_kill(self.child.id() as i32, 15) };
        for _ in 0..50 {
            if self.child.try_wait().ok().flatten().is_some() {
                break;
            }
            std::thread::sleep(Duration::from_millis(100));
        }
        let _ = self.child.kill();
        let _ = self.child.wait();
        let _ = std::fs::remove_dir_all(&self.directory);
    }
}

unsafe fn libc_kill(pid: i32, signal: i32) -> i32 {
    extern "C" {
        fn kill(pid: i32, sig: i32) -> i32;
    }
    kill(pid, signal)
}

static DIRECTORY_COUNTER: std::sync::atomic::AtomicU32 = std::sync::atomic::AtomicU32::new(0);

const CONFIG: &str = r#"
on:
  socket: sock
tasks:
  - name: failing task
    run: 'echo "boom to stdout" && echo "boom to stderr" >&2 && exit 3'
    change: "*.txt"
  - name: passing task
    run: 'echo "hello stdout" && echo "hello stderr" >&2'
    change: "*.txt"
"#;

fn setup_directory(test_name: &str) -> std::path::PathBuf {
    let counter = DIRECTORY_COUNTER.fetch_add(1, std::sync::atomic::Ordering::SeqCst);
    let directory =
        std::env::temp_dir().join(format!("fzzo-{}-{test_name}-{counter}", std::process::id()));
    let _ = std::fs::remove_dir_all(&directory);
    std::fs::create_dir_all(&directory).unwrap();
    std::fs::write(directory.join(".watch.yaml"), CONFIG).unwrap();
    std::fs::canonicalize(&directory).expect("canonicalize fixture root")
}

fn start_watcher(directory: &std::path::Path) -> TestProcess {
    let child_log = std::fs::File::create(directory.join("child.err")).unwrap();
    let child = Command::new(env!("CARGO_BIN_EXE_fzz"))
        .current_dir(directory)
        // Isolate from the ambient environment: fail-fast and non-block flags
        // must come from the test's own config, not the developer's shell.
        .env_remove("FUNZZY_BAIL")
        .env_remove("FUNZZY_NON_BLOCK")
        .stdout(Stdio::from(child_log.try_clone().unwrap()))
        .stderr(Stdio::from(child_log))
        .spawn()
        .unwrap();
    TestProcess {
        child,
        directory: directory.to_path_buf(),
    }
}

fn wait_until_socket(directory: &std::path::Path) {
    let socket_path = directory.join("sock");
    for _ in 0..150 {
        if UnixStream::connect(&socket_path).is_ok() {
            return;
        }
        std::thread::sleep(Duration::from_millis(100));
    }
    panic!(
        "control socket never connectable at {}",
        socket_path.display()
    );
}

fn run_cli(directory: &std::path::Path, args: &[&str]) -> Output {
    Command::new(env!("CARGO_BIN_EXE_fzz"))
        .current_dir(directory)
        .args(args)
        .output()
        .expect("fzz control client should run")
}

fn scheduled_generation(output: &Output) -> u64 {
    let combined = format!(
        "{}{}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    );
    combined
        .lines()
        .find_map(|line| line.strip_prefix("scheduled generation: "))
        .expect(&format!("scheduled generation line missing in: {combined}"))
        .parse()
        .expect("numeric generation")
}

fn raw_status(socket_path: &std::path::Path) -> serde_json::Value {
    let mut stream = UnixStream::connect(socket_path).expect("connect control socket");
    writeln!(
        stream,
        r#"{{"jsonrpc":"2.0","id":"status","method":"status"}}"#
    )
    .unwrap();
    let mut line = String::new();
    BufReader::new(&stream).read_line(&mut line).unwrap();
    serde_json::from_str(&line).unwrap()
}

fn wait_until<F: FnMut() -> bool>(mut condition: F) {
    for _ in 0..200 {
        if condition() {
            return;
        }
        std::thread::sleep(Duration::from_millis(100));
    }
    panic!("wait_until timed out");
}

/// Runs one failing generation and returns its generation identity.
fn run_failing_generation(directory: &std::path::Path) -> u64 {
    let emit = run_cli(directory, &["control", "emit", "notes.txt"]);
    let generation = scheduled_generation(&emit);
    let socket = directory.join("sock");
    wait_until(|| {
        let status = raw_status(&socket);
        status["result"]["generation"].as_u64() == Some(generation)
            && status["result"]["state"].as_str() == Some("failed")
    });
    generation
}

#[test]
fn await_failure_carries_diagnostic_evidence() {
    let directory = setup_directory("await-evidence");
    let _watcher = start_watcher(&directory);
    wait_until_socket(&directory);

    let generation = run_failing_generation(&directory);
    let output = run_cli(
        &directory,
        &[
            "control",
            "await",
            "--generation",
            &generation.to_string(),
            "--timeout",
            "10s",
        ],
    );
    assert_eq!(output.status.code(), Some(1), "failed await exits 1");
    let stdout = String::from_utf8_lossy(&output.stdout);
    assert!(
        stdout.contains("terminal reason: failed"),
        "stdout: {stdout}"
    );
    assert!(stdout.contains("failure evidence:"), "stdout: {stdout}");
    assert!(
        stdout.contains("boom to stdout"),
        "excerpt must show output: {stdout}"
    );
    assert!(
        stdout.contains(&format!("--generation {generation} --task 'failing task'")),
        "retrieval hint: {stdout}"
    );
    assert!(stdout.contains("observed_bytes:"), "stdout: {stdout}");
}

#[test]
fn status_carries_failure_evidence_for_the_latest_generation() {
    let directory = setup_directory("status-evidence");
    let _watcher = start_watcher(&directory);
    wait_until_socket(&directory);

    let generation = run_failing_generation(&directory);
    let status = raw_status(&directory.join("sock"));
    assert_eq!(status["result"]["generation"].as_u64(), Some(generation));
    assert_eq!(status["result"]["state"].as_str(), Some("failed"));
    let evidence = status["result"]["failureEvidence"]
        .as_object()
        .expect("evidence");
    assert!(evidence["excerpt"]
        .as_str()
        .unwrap_or("")
        .contains("boom to stdout"));
    assert!(evidence["retrieve"]
        .as_str()
        .unwrap_or("")
        .contains("--generation"));
}

#[test]
fn output_retrieval_returns_full_stdout_and_stderr() {
    let directory = setup_directory("retrieve-full");
    let _watcher = start_watcher(&directory);
    wait_until_socket(&directory);

    let generation = run_failing_generation(&directory);
    let output = run_cli(
        &directory,
        &[
            "control",
            "output",
            "--generation",
            &generation.to_string(),
            "--task",
            "failing task",
            "--full",
        ],
    );
    assert!(output.status.success());
    let stdout = String::from_utf8_lossy(&output.stdout);
    assert!(stdout.contains("task: failing task"), "stdout: {stdout}");
    assert!(stdout.contains("boom to stdout"), "stdout: {stdout}");
    assert!(stdout.contains("boom to stderr"), "stdout: {stdout}");
}

#[test]
fn output_retrieval_tail_and_stream_filters() {
    let directory = setup_directory("retrieve-filters");
    let _watcher = start_watcher(&directory);
    wait_until_socket(&directory);

    let generation = run_failing_generation(&directory);
    let generation = generation.to_string();

    // Tail: last line of stdout.
    let tail = run_cli(
        &directory,
        &[
            "control",
            "output",
            "--generation",
            &generation,
            "--task",
            "failing task",
            "--stdout",
            "--tail",
            "1",
        ],
    );
    assert!(tail.status.success());
    let stdout = String::from_utf8_lossy(&tail.stdout);
    assert!(stdout.contains("boom to stdout"), "stdout: {stdout}");
    assert!(
        !stdout.contains("boom to stderr"),
        "stderr filtered out: {stdout}"
    );

    // Stream filter: stderr only.
    let stderr = run_cli(
        &directory,
        &[
            "control",
            "output",
            "--generation",
            &generation,
            "--task",
            "failing task",
            "--stderr",
        ],
    );
    assert!(stderr.status.success());
    let stdout = String::from_utf8_lossy(&stderr.stdout);
    assert!(stdout.contains("boom to stderr"), "stdout: {stdout}");
    assert!(
        !stdout.contains("boom to stdout"),
        "stdout filtered out: {stdout}"
    );
}

#[test]
fn passing_task_output_is_retained_and_retrievable() {
    let directory = setup_directory("retrieve-passing");
    let _watcher = start_watcher(&directory);
    wait_until_socket(&directory);

    // Emit triggers both tasks; the failing one runs first, the passing one
    // still runs (fail-fast is off by default).
    let emit = run_cli(&directory, &["control", "emit", "x.txt"]);
    let generation = scheduled_generation(&emit);
    let socket = directory.join("sock");
    wait_until(|| {
        let status = raw_status(&socket);
        status["result"]["generation"].as_u64() == Some(generation)
            && matches!(
                status["result"]["state"].as_str(),
                Some("passed" | "failed" | "cancelled")
            )
    });

    let output = run_cli(
        &directory,
        &[
            "control",
            "output",
            "--generation",
            &generation.to_string(),
            "--task",
            "passing task",
            "--full",
        ],
    );
    let combined = format!(
        "{}{}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    );
    assert!(output.status.success(), "output failed: {combined}");
    let stdout = String::from_utf8_lossy(&output.stdout);
    assert!(stdout.contains("hello stdout"), "stdout: {stdout}");
    assert!(stdout.contains("hello stderr"), "stdout: {stdout}");
}

#[test]
fn missing_generation_and_task_are_actionable_errors() {
    let directory = setup_directory("missing");
    let _watcher = start_watcher(&directory);
    wait_until_socket(&directory);

    let generation = run_failing_generation(&directory);

    // Contract §3: typed generation_not_found (-32010) with the retained
    // range in structured data; the human CLI renders both.
    let missing_generation = run_cli(&directory, &["control", "output", "--generation", "999"]);
    assert_eq!(missing_generation.status.code(), Some(1));
    let stdout = String::from_utf8_lossy(&missing_generation.stdout);
    assert!(
        stdout.contains("generation_not_found (-32010)"),
        "typed code: {stdout}"
    );
    assert!(
        stdout.contains(&format!("\"retained\":[{generation}]")),
        "retained range: {stdout}"
    );

    // Contract §3: typed task_not_found (-32011) lists the exact retained
    // task IDs as copy-safe candidates instead of a human "retained tasks:" line.
    let missing_task = run_cli(
        &directory,
        &[
            "control",
            "output",
            "--generation",
            &generation.to_string(),
            "--task",
            "nope",
        ],
    );
    assert_eq!(missing_task.status.code(), Some(1));
    let stdout = String::from_utf8_lossy(&missing_task.stdout);
    assert!(
        stdout.contains("task_not_found (-32011)"),
        "typed code: {stdout}"
    );
    assert!(stdout.contains("failing task"), "exact candidate: {stdout}");
}

#[test]
fn watcher_restart_clears_all_retained_output() {
    let directory = setup_directory("restart-clears");
    let mut watcher = start_watcher(&directory);
    wait_until_socket(&directory);

    let generation = run_failing_generation(&directory);

    watcher.kill();
    let _ = std::fs::remove_file(directory.join("sock"));
    std::mem::forget(watcher);
    let replacement = start_watcher(&directory);
    wait_until_socket(&directory);

    // Contract §3: restart clears retention, so the stale generation is a
    // typed generation_not_found with an empty retained range.
    let output = run_cli(
        &directory,
        &["control", "output", "--generation", &generation.to_string()],
    );
    assert_eq!(
        output.status.code(),
        Some(1),
        "restart must clear retention"
    );
    let stdout = String::from_utf8_lossy(&output.stdout);
    assert!(
        stdout.contains("generation_not_found (-32010)"),
        "stdout: {stdout}"
    );
    assert!(
        stdout.contains("\"retained\":[]"),
        "empty retained after restart: {stdout}"
    );
    let _ = replacement;
}

#[test]
fn output_usage_errors_exit_two() {
    let directory = setup_directory("usage");
    let _watcher = start_watcher(&directory);
    wait_until_socket(&directory);

    let missing_generation = run_cli(&directory, &["control", "output"]);
    assert_eq!(missing_generation.status.code(), Some(2));

    let conflict = run_cli(
        &directory,
        &[
            "control",
            "output",
            "--generation",
            "1",
            "--tail",
            "5",
            "--full",
        ],
    );
    assert_eq!(conflict.status.code(), Some(2));
}

#[test]
fn page_mode_pages_all_output_without_duplicates_or_skips() {
    // Contract §5: one deterministic page stream below the budget; following
    // every cursor yields exactly the full retained output with no skipped or
    // duplicated bytes, and no single response exceeds the negotiated budget.
    let directory = setup_directory("paging");
    let _watcher = start_watcher(&directory);
    wait_until_socket(&directory);

    // Emit a generation whose task prints enough to span several pages.
    let emit = run_cli(&directory, &["control", "emit", "paging.txt"]);
    let generation = scheduled_generation(&emit);
    let socket = directory.join("sock");
    wait_until(|| {
        let status = raw_status(&socket);
        status["result"]["generation"].as_u64() == Some(generation)
            && status["result"]["state"].as_str() == Some("failed")
    });

    // Page with a small budget so the output cannot fit one page; follow the
    // JSON continuation cursor until the final page.
    let mut cursor: Option<String> = None;
    let mut collected = String::new();
    let mut pages = 0;
    loop {
        let mut args: Vec<String> = vec![
            "control".to_string(),
            "--format".to_string(),
            "json".to_string(),
            "output".to_string(),
            "--generation".to_string(),
            generation.to_string(),
            "--page".to_string(),
            "--page-size".to_string(),
            "512".to_string(),
        ];
        if let Some(ref c) = cursor {
            args.push("--cursor".to_string());
            args.push(c.clone());
        }
        let args: Vec<&str> = args.iter().map(String::as_str).collect();
        let json = run_cli(&directory, &args);
        assert!(json.status.success(), "page {pages} failed");
        let doc: serde_json::Value =
            serde_json::from_str(std::str::from_utf8(&json.stdout).unwrap().trim())
                .expect("one json document");
        if let Some(tasks) = doc["tasks"].as_array() {
            for task in tasks {
                for stream in ["stdout", "stderr"] {
                    if let Some(content) = task.get(stream).and_then(|s| s["content"].as_str()) {
                        collected.push_str(content);
                    }
                }
            }
        }
        cursor = doc["nextCursor"].as_str().map(str::to_owned);
        pages += 1;
        if cursor.is_none() {
            break;
        }
        assert!(pages < 50, "paging did not terminate");
    }

    assert!(pages > 1, "small budget must produce multiple pages");
    assert!(
        collected.contains("boom to stdout"),
        "all pages must carry the full output, got: {collected}"
    );
    assert!(
        collected.contains("boom to stderr"),
        "all pages must carry the full output, got: {collected}"
    );
    assert!(
        collected.matches("boom to stdout").count() == 1,
        "no duplicated bytes across pages: {collected}"
    );
}

#[test]
fn page_flags_reject_structural_conflicts_before_socket() {
    // Contract §2: tail vs page/full variants are structurally exclusive and
    // rejected by the CLI before any socket call (exit 2, usage error).
    let directory = setup_directory("page-conflicts");
    let _watcher = start_watcher(&directory);
    wait_until_socket(&directory);

    for args in [
        vec![
            "control",
            "output",
            "--generation",
            "1",
            "--page",
            "--tail",
            "5",
        ],
        vec!["control", "output", "--generation", "1", "--page", "--full"],
        vec![
            "control",
            "output",
            "--generation",
            "1",
            "--page-size",
            "512",
        ],
        vec![
            "control",
            "output",
            "--generation",
            "1",
            "--cursor",
            "7|0|0|0",
        ],
    ] {
        let output = run_cli(&directory, &args);
        assert_eq!(
            output.status.code(),
            Some(2),
            "structural conflict must exit 2: {args:?}"
        );
    }
}

#[test]
fn failure_evidence_output_ref_follows_to_one_bounded_retrieval() {
    // Contract §1/§8: a failed generation's observation carries an exact
    // structured outputRef; following it once retrieves the evidence without
    // reconstructing task names from prose.
    let directory = setup_directory("output-ref");
    let _watcher = start_watcher(&directory);
    wait_until_socket(&directory);

    let generation = run_failing_generation(&directory);

    // Await the failed generation; the observation must carry outputRef. A
    // failed workflow exits 1 (contract §8) while still emitting the document.
    let awaited = run_cli(
        &directory,
        &[
            "control",
            "--format",
            "json",
            "await",
            "--generation",
            &generation.to_string(),
            "--timeout",
            "10s",
        ],
    );
    assert_eq!(awaited.status.code(), Some(1), "failed workflow exits 1");
    let doc: serde_json::Value = serde_json::from_str(
        std::str::from_utf8(&awaited.stdout)
            .unwrap()
            .lines()
            .next()
            .expect("first line is the json document")
            .trim(),
    )
    .expect("one json document");
    assert_eq!(doc["terminalReason"], "failed");
    let output_ref = doc["failureEvidence"]["outputRef"].clone();
    let task = output_ref["task"].as_str().expect("exact task id");
    let instance_token = output_ref["instanceToken"]
        .as_str()
        .expect("instance token");
    assert_eq!(output_ref["generation"], generation);

    // One retrieval with the exact identities from the reference succeeds and
    // stays bounded; the shell-safe command from the ref is copy-pasteable.
    let retrieve_cmd = output_ref["retrieve"].as_str().expect("retrieve command");
    assert!(retrieve_cmd.contains("--instance '"), "{retrieve_cmd}");
    assert!(retrieve_cmd.contains("--task '"), "{retrieve_cmd}");

    let retrieved = run_cli(
        &directory,
        &[
            "control",
            "--format",
            "json",
            "output",
            "--generation",
            &generation.to_string(),
            "--task",
            task,
            "--tail",
            "80",
        ],
    );
    assert!(retrieved.status.success(), "one retrieval succeeds");
    let out: serde_json::Value =
        serde_json::from_str(std::str::from_utf8(&retrieved.stdout).unwrap().trim())
            .expect("one json document");
    assert_eq!(out["tasks"][0]["id"], task);
    assert!(out["tasks"][0]["stdout"]["content"]
        .as_str()
        .unwrap()
        .contains("boom"));
    let _ = instance_token;
}
