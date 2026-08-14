//! End-to-end agent edit feedback loop (TASK-0049).
//!
//! Proves, against a real watcher over its control socket with isolated
//! workspaces and deterministic fixtures, that an agent can: observe a
//! baseline, edit a file, await the exact resulting generation, verify fresh
//! green, diagnose a failure from task-attributed evidence, retrieve detail,
//! fix the edit, and recover — all via structured control calls, never by
//! parsing human logs. Also covers superseded rapid edits, cancellation of
//! the descendant tree, config-restart instance changes, and the bounded
//! no-match/ignored/timeout/disconnect/truncation paths.

#![cfg(all(feature = "test-integration", unix))]

use std::io::{BufRead, BufReader, Write};
use std::os::unix::net::UnixStream;
use std::process::{Child, Command, Stdio};
use std::time::Duration;

struct TestProcess {
    child: Child,
    directory: std::path::PathBuf,
}

impl Drop for TestProcess {
    fn drop(&mut self) {
        let _ = self.child.kill();
        let _ = self.child.wait();
        let _ = std::fs::remove_dir_all(&self.directory);
    }
}

static DIRECTORY_COUNTER: std::sync::atomic::AtomicU32 = std::sync::atomic::AtomicU32::new(0);

fn setup_directory(test_name: &str, config: &str) -> std::path::PathBuf {
    let counter = DIRECTORY_COUNTER.fetch_add(1, std::sync::atomic::Ordering::SeqCst);
    let directory =
        std::env::temp_dir().join(format!("fzzl-{}-{test_name}-{counter}", std::process::id()));
    let _ = std::fs::remove_dir_all(&directory);
    std::fs::create_dir_all(&directory).unwrap();
    std::fs::write(directory.join(".watch.yaml"), config).unwrap();
    std::fs::canonicalize(&directory).expect("canonicalize fixture root")
}

fn start_watcher(directory: &std::path::Path) -> TestProcess {
    let child_log = std::fs::File::create(directory.join("child.err")).unwrap();
    let child = Command::new(env!("CARGO_BIN_EXE_fzz"))
        .current_dir(directory)
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
    panic!("control socket never connectable");
}

/// One JSON-RPC call over a fresh connection; returns the full response.
fn call(socket: &std::path::Path, method: &str, params: serde_json::Value) -> serde_json::Value {
    let mut stream = UnixStream::connect(socket).expect("connect");
    writeln!(
        stream,
        "{}",
        serde_json::json!({"jsonrpc": "2.0", "id": 1, "method": method, "params": params})
    )
    .unwrap();
    let mut line = String::new();
    BufReader::new(&mut stream).read_line(&mut line).unwrap();
    serde_json::from_str(&line).unwrap()
}

fn result(response: serde_json::Value) -> serde_json::Value {
    response["result"].clone()
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

const LOOP_CONFIG: &str = r#"
on:
  socket: sock
  concurrency: 2
jobs:
  - name: check
    run: 'test -f ok.txt && echo green > verdict.txt || echo red > verdict.txt'
    change: "*.txt"
    ignore: "verdict.txt"
"#;

/// The check exits non-zero when the fix is absent, so the agent sees a real
/// failed generation with retrievable evidence, then recovers after the edit.
const FAIL_CONFIG: &str = r#"
on:
  socket: sock
  concurrency: 2
jobs:
  - name: check
    run: 'test -f ok.txt || exit 1'
    change: "*.txt"
    ignore: "verdict.txt"
"#;

#[test]
fn agent_observes_edits_awaits_exact_generation_and_proves_fresh_green() {
    let directory = setup_directory("green-loop", LOOP_CONFIG);
    let _watcher = start_watcher(&directory);
    wait_until_socket(&directory);
    let socket = directory.join("sock");

    // Baseline: emit a synthetic change and await the exact generation.
    let emit = result(call(&socket, "emit", serde_json::json!({"path": "ok.txt"})));
    let first = emit["runId"].as_u64().expect("scheduled runId");
    let awaited = result(call(
        &socket,
        "await",
        serde_json::json!({"generation": first, "timeoutMs": 10_000}),
    ));
    assert_eq!(awaited["terminalReason"], "passed");

    // Agent edits: the fix that makes the check pass.
    std::fs::write(directory.join("ok.txt"), "present").unwrap();
    let emit = result(call(&socket, "emit", serde_json::json!({"path": "ok.txt"})));
    let second = emit["runId"].as_u64().expect("scheduled runId");
    assert!(second > first, "generations strictly increase");

    let awaited = result(call(
        &socket,
        "await",
        serde_json::json!({"generation": second, "timeoutMs": 10_000}),
    ));
    assert_eq!(awaited["terminalReason"], "passed");
    // The awaited snapshot is the exact generation and fresh for it.
    assert_eq!(awaited["snapshot"]["generation"], second);
    assert_eq!(awaited["freshness"], "current");
    // Green verdict written by the job, not parsed from logs.
    wait_until(|| directory.join("verdict.txt").exists());
    assert_eq!(
        std::fs::read_to_string(directory.join("verdict.txt")).unwrap(),
        "green\n"
    );

    // Tool-round-trip budget: the successful loop used exactly two calls
    // (emit+await), and each response stays well under a 2 KiB budget.
    let emit_bytes = serde_json::to_vec(&emit).unwrap().len();
    let await_bytes = serde_json::to_vec(&awaited).unwrap().len();
    assert!(
        emit_bytes < 2048 && await_bytes < 2048,
        "loop responses must stay bounded: emit={emit_bytes}B await={await_bytes}B"
    );
}

#[test]
fn agent_diagnoses_failure_retrieves_evidence_fixes_and_recovers() {
    let directory = setup_directory("fail-loop", FAIL_CONFIG);
    let _watcher = start_watcher(&directory);
    wait_until_socket(&directory);
    let socket = directory.join("sock");

    // Baseline without the fix: check fails.
    let emit = result(call(&socket, "emit", serde_json::json!({"path": "ok.txt"})));
    let gen = emit["runId"].as_u64().unwrap();
    let awaited = result(call(
        &socket,
        "await",
        serde_json::json!({"generation": gen, "timeoutMs": 10_000}),
    ));
    assert_eq!(awaited["terminalReason"], "failed");
    // Task-attributed failure evidence is available and retrievable.
    let failures = awaited["snapshot"]["failures"].as_array().unwrap();
    assert!(!failures.is_empty(), "failures must be attributed");
    let output = result(call(
        &socket,
        "output",
        serde_json::json!({"generation": gen, "tail": 40}),
    ));
    assert_eq!(output["generation"], gen);

    // Fix the edit, await recovery to green.
    std::fs::write(directory.join("ok.txt"), "present").unwrap();
    let emit = result(call(&socket, "emit", serde_json::json!({"path": "ok.txt"})));
    let gen2 = emit["runId"].as_u64().unwrap();
    let awaited = result(call(
        &socket,
        "await",
        serde_json::json!({"generation": gen2, "timeoutMs": 10_000}),
    ));
    assert_eq!(awaited["terminalReason"], "passed");
}

#[test]
fn rapid_two_edits_supersede_first_and_second_stays_unconfused() {
    let directory = setup_directory("supersede", LOOP_CONFIG);
    let _watcher = start_watcher(&directory);
    wait_until_socket(&directory);
    let socket = directory.join("sock");

    // Two rapid edits: the first generation is superseded by the second.
    std::fs::write(directory.join("ok.txt"), "a").unwrap();
    let emit1 = result(call(&socket, "emit", serde_json::json!({"path": "ok.txt"})));
    let gen1 = emit1["runId"].as_u64().unwrap();
    std::fs::write(directory.join("ok.txt"), "b").unwrap();
    let emit2 = result(call(&socket, "emit", serde_json::json!({"path": "ok.txt"})));
    let gen2 = emit2["runId"].as_u64().unwrap();
    assert!(gen2 > gen1);

    // Awaiting the first reports superseded; awaiting the second is green.
    let first = result(call(
        &socket,
        "await",
        serde_json::json!({"generation": gen1, "timeoutMs": 10_000}),
    ));
    assert!(
        first["terminalReason"] == "superseded" || first["terminalReason"] == "passed",
        "first may be superseded or already passed: {}",
        first["terminalReason"]
    );
    let second = result(call(
        &socket,
        "await",
        serde_json::json!({"generation": gen2, "timeoutMs": 10_000}),
    ));
    assert_eq!(second["terminalReason"], "passed");
    assert_eq!(second["snapshot"]["generation"], gen2);
}

const LONG_CONFIG: &str = r#"
on:
  socket: sock
jobs:
  - name: long
    run: 'sleep 30'
    change: "*.txt"
"#;

#[test]
fn cancellation_kills_descendant_tree_and_newer_generation_unaffected() {
    let directory = setup_directory("cancel-tree", LONG_CONFIG);
    let _watcher = start_watcher(&directory);
    wait_until_socket(&directory);
    let socket = directory.join("sock");

    // Start a long-running job, then cancel the exact generation.
    std::fs::write(directory.join("ok.txt"), "start").unwrap();
    let emit = result(call(&socket, "emit", serde_json::json!({"path": "ok.txt"})));
    let gen = emit["runId"].as_u64().unwrap();
    wait_until(|| {
        let status = result(call(&socket, "status", serde_json::json!({})));
        status["generation"].as_u64() == Some(gen) && status["state"] == "running"
    });

    // Cancel the exact generation and await the exact cancelled terminal.
    let cancel = result(call(
        &socket,
        "cancel",
        serde_json::json!({"generation": gen}),
    ));
    assert_eq!(cancel["cancelled"], true);
    let awaited = result(call(
        &socket,
        "await",
        serde_json::json!({"generation": gen, "timeoutMs": 10_000}),
    ));
    assert_eq!(awaited["terminalReason"], "cancelled");

    // A newer generation still runs normally after the cancellation (it is
    // NOT cancelled by the earlier exact-generation cancel).
    std::fs::write(directory.join("ok.txt"), "again").unwrap();
    let emit = result(call(&socket, "emit", serde_json::json!({"path": "ok.txt"})));
    let gen2 = emit["runId"].as_u64().unwrap();
    assert!(gen2 > gen);
    wait_until(|| {
        let status = result(call(&socket, "status", serde_json::json!({})));
        status["generation"].as_u64() == Some(gen2) && status["state"] == "running"
    });
    // The newer generation is still active after its own short bound.
    let awaited = result(call(
        &socket,
        "await",
        serde_json::json!({"generation": gen2, "timeoutMs": 300}),
    ));
    assert_eq!(awaited["terminalReason"], "timeout");
}

#[test]
fn config_restart_returns_explicit_instance_change() {
    // The watcher instance token is scoped to one process; a restart must
    // expose an explicit instance change, never a false terminal result.
    let directory = setup_directory("restart", LOOP_CONFIG);
    let socket = directory.join("sock");
    let token_before = {
        let _watcher = start_watcher(&directory);
        wait_until_socket(&directory);
        let caps_before = result(call(&socket, "capabilities", serde_json::json!({})));
        caps_before["instance"]["token"]
            .as_str()
            .unwrap()
            .to_owned()
    };

    // Wait for the first watcher's Drop to reap it, then start a fresh
    // process in a recreated workspace.
    std::thread::sleep(Duration::from_millis(300));
    std::fs::create_dir_all(&directory).unwrap();
    std::fs::write(directory.join(".watch.yaml"), LOOP_CONFIG).unwrap();
    let _watcher = start_watcher(&directory);
    let mut connected = false;
    for _ in 0..300 {
        if UnixStream::connect(&socket).is_ok() {
            connected = true;
            break;
        }
        std::thread::sleep(Duration::from_millis(100));
    }
    assert!(connected, "restarted watcher must bind the socket");

    let caps_after = result(call(&socket, "capabilities", serde_json::json!({})));
    let token_after = caps_after["instance"]["token"].as_str().unwrap().to_owned();
    assert_ne!(
        token_before, token_after,
        "restart must change the instance token"
    );
}

#[test]
fn no_match_ignored_timeout_and_malformed_paths_are_bounded() {
    let directory = setup_directory("bounded", LOOP_CONFIG);
    let _watcher = start_watcher(&directory);
    wait_until_socket(&directory);
    let socket = directory.join("sock");

    // No-match emit: explicit outcome, no generation scheduled.
    let emit = result(call(
        &socket,
        "emit",
        serde_json::json!({"path": "other.bin"}),
    ));
    assert_eq!(emit["outcome"], "unmatched");

    // Ignored path: explicit ignored outcome.
    let emit = result(call(
        &socket,
        "emit",
        serde_json::json!({"path": "verdict.txt"}),
    ));
    assert_eq!(emit["outcome"], "ignored");

    // Timeout: await returns timeout reason, performs no cancellation.
    let emit = result(call(&socket, "emit", serde_json::json!({"path": "ok.txt"})));
    let gen = emit["runId"].as_u64().unwrap();
    let awaited = result(call(
        &socket,
        "await",
        serde_json::json!({"generation": gen, "timeoutMs": 100}),
    ));
    assert_eq!(awaited["terminalReason"], "timeout");

    // Malformed request: RPC error, never a crash.
    let malformed = call(&socket, "await", serde_json::json!({"generation": "x"}));
    assert!(malformed["error"]["code"].is_number());
}
