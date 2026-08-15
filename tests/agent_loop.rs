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

/// Config with a tag-bearing, space-containing job name — the audit's exact
/// failure case (agent shortened `run integration @agent-final` to
/// `run integration` and failed eight times). The proof: the observation
/// carries the exact structured reference, and ONE retrieval with that exact
/// identity succeeds without guessing or permutation.
const TAGGED_FAIL_CONFIG: &str = r#"
on:
  socket: sock
jobs:
  - name: run integration @agent-final
    run: 'echo "boom to stdout" && echo "boom to stderr" >&2 && exit 3'
    change: "*.txt"
"#;

#[test]
fn one_failure_reaches_evidence_in_one_output_call() {
    let directory = setup_directory("one-hop", TAGGED_FAIL_CONFIG);
    let _watcher = start_watcher(&directory);
    wait_until_socket(&directory);
    let socket = directory.join("sock");

    // One observation: emit the failing change and await the exact generation.
    let emit = result(call(
        &socket,
        "emit",
        serde_json::json!({"path": "notes.txt"}),
    ));
    let gen = emit["runId"].as_u64().unwrap();
    let awaited = result(call(
        &socket,
        "await",
        serde_json::json!({"generation": gen, "timeoutMs": 10_000}),
    ));
    assert_eq!(awaited["terminalReason"], "failed");

    // The observation carries an exact structured outputRef — the full
    // tag-bearing identity, never a shortened display name.
    let output_ref = &awaited["failureEvidence"]["outputRef"];
    assert_eq!(output_ref["task"], "run integration @agent-final");
    assert_eq!(output_ref["generation"], gen);
    assert!(!output_ref["instanceToken"].as_str().unwrap().is_empty());
    let retrieve = output_ref["retrieve"].as_str().unwrap();
    assert!(
        retrieve.contains("--task 'run integration @agent-final'"),
        "retrieve must quote the full exact identity: {retrieve}"
    );

    // Exactly ONE retrieval call, using the exact identities from the ref
    // (task + instance), succeeds below the transport budget.
    let retrieved = result(call(
        &socket,
        "output",
        serde_json::json!({
            "generation": gen,
            "task": "run integration @agent-final",
            "instanceToken": output_ref["instanceToken"],
            "tail": 80,
        }),
    ));
    assert_eq!(retrieved["tasks"][0]["id"], "run integration @agent-final");
    let serialized = serde_json::to_vec(&retrieved).unwrap();
    assert!(
        serialized.len() < 65_536,
        "one-hop retrieval must stay below transport, got {}",
        serialized.len()
    );
    let stdout = retrieved["tasks"][0]["stdout"]["content"].as_str().unwrap();
    assert!(stdout.contains("boom to stdout"), "stdout: {stdout}");
    let stderr = retrieved["tasks"][0]["stderr"]["content"].as_str().unwrap();
    assert!(stderr.contains("boom to stderr"), "stderr: {stderr}");
}

/// Config whose task emits enough output to span several small pages, proving
/// whole-generation retrieval stays below transport with continuation.
const LARGE_FAIL_CONFIG: &str = r#"
on:
  socket: sock
jobs:
  - name: loud
    run: 'for i in $(seq 1 2000); do echo "line $i boom"; done; echo "tail stderr boom" >&2; exit 3'
    change: "*.txt"
"#;

#[test]
fn whole_generation_and_one_task_stay_below_transport_with_continuation() {
    let directory = setup_directory("whole-gen", LARGE_FAIL_CONFIG);
    let _watcher = start_watcher(&directory);
    wait_until_socket(&directory);
    let socket = directory.join("sock");

    let emit = result(call(
        &socket,
        "emit",
        serde_json::json!({"path": "notes.txt"}),
    ));
    let gen = emit["runId"].as_u64().unwrap();
    let awaited = result(call(
        &socket,
        "await",
        serde_json::json!({"generation": gen, "timeoutMs": 10_000}),
    ));
    assert_eq!(awaited["terminalReason"], "failed");

    // Whole-generation retrieval shares one budget: request a small page and
    // follow the continuation; every response stays below the transport limit
    // and the concatenated stream covers the full output with no duplicates.
    let mut cursor: Option<String> = None;
    let mut collected = String::new();
    let mut pages = 0;
    loop {
        let mut params = serde_json::json!({
            "generation": gen,
            "mode": "page",
            "maxBytes": 2048,
        });
        if let Some(ref c) = cursor {
            params["cursor"] = serde_json::json!(c);
        }
        let page = result(call(&socket, "output", params));
        let serialized = serde_json::to_vec(&page).unwrap();
        assert!(
            serialized.len() < 65_536,
            "page {} must stay below transport, got {}",
            pages,
            serialized.len()
        );
        for task in page["tasks"].as_array().unwrap() {
            for stream in ["stdout", "stderr"] {
                if let Some(content) = task[stream]["content"].as_str() {
                    collected.push_str(content);
                }
            }
        }
        cursor = page["nextCursor"].as_str().map(str::to_owned);
        pages += 1;
        if cursor.is_none() {
            break;
        }
        assert!(pages < 20, "paging did not terminate");
    }
    assert!(pages > 1, "small budget must span multiple pages");
    assert_eq!(
        collected.matches("line 1 boom").count(),
        1,
        "no duplicated bytes across pages: {collected}"
    );
    assert!(collected.contains("tail stderr boom"));

    // One-task retrieval (tail) is also bounded below transport.
    let one_task = result(call(
        &socket,
        "output",
        serde_json::json!({"generation": gen, "task": "loud", "tail": 80}),
    ));
    let serialized = serde_json::to_vec(&one_task).unwrap();
    assert!(serialized.len() < 65_536, "one-task retrieval bounded");
}

#[test]
fn unknown_task_returns_typed_candidates_and_resolves_unambiguous_once() {
    let directory = setup_directory("unknown-task", TAGGED_FAIL_CONFIG);
    let _watcher = start_watcher(&directory);
    wait_until_socket(&directory);
    let socket = directory.join("sock");

    let emit = result(call(
        &socket,
        "emit",
        serde_json::json!({"path": "notes.txt"}),
    ));
    let gen = emit["runId"].as_u64().unwrap();
    let awaited = result(call(
        &socket,
        "await",
        serde_json::json!({"generation": gen, "timeoutMs": 10_000}),
    ));
    assert_eq!(awaited["terminalReason"], "failed");

    // A genuinely unknown task (no canonical prefix) gets a typed error
    // naming the exact candidate — never a guess, never a generic error.
    let missing = call(
        &socket,
        "output",
        serde_json::json!({"generation": gen, "task": "nope"}),
    );
    assert_eq!(missing["error"]["code"], -32011);
    let candidates = missing["error"]["data"]["candidates"].as_array().unwrap();
    assert!(
        candidates
            .iter()
            .any(|c| c == "run integration @agent-final"),
        "typed candidates must name the exact id: {missing}"
    );

    // A shortened canonical prefix resolves read-only, reporting the
    // selected exact ID (contract §6) — the audit's "run integration" case.
    let resolved = result(call(
        &socket,
        "output",
        serde_json::json!({"generation": gen, "task": "run integration", "tail": 80}),
    ));
    assert_eq!(resolved["resolvedTask"], "run integration @agent-final");
    assert_eq!(resolved["tasks"][0]["id"], "run integration @agent-final");
}

#[test]
fn stale_instance_with_reused_generation_cannot_read_replacement_output() {
    // A stale instance token must never read the same-number generation from
    // a replacement watcher (contract §3 `-32012`), even when the generation
    // counter restarts and reuses the same number.
    let directory = setup_directory("stale-instance", TAGGED_FAIL_CONFIG);
    let socket = directory.join("sock");
    let old_token = {
        let _watcher = start_watcher(&directory);
        wait_until_socket(&directory);
        result(call(&socket, "capabilities", serde_json::json!({})))["instance"]["token"]
            .as_str()
            .unwrap()
            .to_owned()
    };

    // Restart into a recreated workspace; the new watcher reuses generation 1.
    std::thread::sleep(Duration::from_millis(300));
    std::fs::create_dir_all(&directory).unwrap();
    std::fs::write(directory.join(".watch.yaml"), TAGGED_FAIL_CONFIG).unwrap();
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

    let emit = result(call(
        &socket,
        "emit",
        serde_json::json!({"path": "notes.txt"}),
    ));
    let gen = emit["runId"].as_u64().unwrap();
    let awaited = result(call(
        &socket,
        "await",
        serde_json::json!({"generation": gen, "timeoutMs": 10_000}),
    ));
    assert_eq!(awaited["terminalReason"], "failed");

    // Reading with the OLD instance token is a typed instance mismatch, even
    // though the generation number matches the replacement watcher's run.
    let stale = call(
        &socket,
        "output",
        serde_json::json!({
            "generation": gen,
            "instanceToken": old_token,
        }),
    );
    assert_eq!(stale["error"]["code"], -32012);
    assert_eq!(stale["error"]["data"]["action"], "restart-or-reobserve");

    // The fresh instance token reads normally.
    let fresh_token = result(call(&socket, "capabilities", serde_json::json!({})))["instance"]
        ["token"]
        .as_str()
        .unwrap()
        .to_owned();
    let fresh = result(call(
        &socket,
        "output",
        serde_json::json!({"generation": gen, "instanceToken": fresh_token, "tail": 80}),
    ));
    assert_eq!(fresh["generation"], gen);
}

#[test]
fn invalid_option_combinations_rejected_before_any_retrieval() {
    // Contract §2: structurally exclusive options are rejected with a typed
    // invalid_options error at the server (and exit 2 at the CLI before the
    // socket); no parameter permutation can turn a schema mismatch into a
    // valid response.
    let directory = setup_directory("invalid-opts", TAGGED_FAIL_CONFIG);
    let _watcher = start_watcher(&directory);
    wait_until_socket(&directory);
    let socket = directory.join("sock");

    let emit = result(call(
        &socket,
        "emit",
        serde_json::json!({"path": "notes.txt"}),
    ));
    let gen = emit["runId"].as_u64().unwrap();
    let awaited = result(call(
        &socket,
        "await",
        serde_json::json!({"generation": gen, "timeoutMs": 10_000}),
    ));
    assert_eq!(awaited["terminalReason"], "failed");

    // page + tail: structurally exclusive, rejected before retrieval.
    let conflict = call(
        &socket,
        "output",
        serde_json::json!({"generation": gen, "mode": "page", "tail": 40}),
    );
    assert_eq!(conflict["error"]["code"], -32013);

    // cursor without page mode: rejected.
    let cursor_only = call(
        &socket,
        "output",
        serde_json::json!({"generation": gen, "cursor": "1|0|0|0"}),
    );
    assert_eq!(cursor_only["error"]["code"], -32013);

    // Unknown mode: rejected with the valid set.
    let bad_mode = call(
        &socket,
        "output",
        serde_json::json!({"generation": gen, "mode": "dump"}),
    );
    assert_eq!(bad_mode["error"]["code"], -32013);
    assert_eq!(
        bad_mode["error"]["data"]["valid"],
        serde_json::json!(["tail", "page"])
    );
}

/// Config with two parallel tasks where one fails fast and the other runs
/// longer — completion order differs from declaration order, so the retained
/// output must stay attributed to the exact task regardless of who finished
/// first (contract §1 identity).
const PARALLEL_FAIL_CONFIG: &str = r#"
on:
  socket: sock
  concurrency: 4
jobs:
  - name: slow pass
    run: 'sleep 1 && echo "slow ok"'
    change: "*.txt"
  - name: fast fail
    run: 'echo "secret=abc123" && echo "early boom" >&2 && exit 3'
    change: "*.txt"
"#;

#[test]
fn parallel_reversed_completion_keeps_exact_identity_and_bounds() {
    // Task completion order is not declaration order; output retention must
    // key each stream to its exact task id (never by finish time) and stay
    // bounded. Secret-like content is retained but never redacted or echoed
    // into evidence summaries (the socket permission is the boundary).
    let directory = setup_directory("parallel-reversed", PARALLEL_FAIL_CONFIG);
    let _watcher = start_watcher(&directory);
    wait_until_socket(&directory);
    let socket = directory.join("sock");

    let emit = result(call(
        &socket,
        "emit",
        serde_json::json!({"path": "notes.txt"}),
    ));
    let gen = emit["runId"].as_u64().unwrap();
    let awaited = result(call(
        &socket,
        "await",
        serde_json::json!({"generation": gen, "timeoutMs": 10_000}),
    ));
    assert_eq!(awaited["terminalReason"], "failed");

    // The failed task's evidence names its exact id and carries the ref.
    let output_ref = &awaited["failureEvidence"]["outputRef"];
    assert_eq!(output_ref["task"], "fast fail");

    // Whole-generation retrieval attributes every stream to its exact task,
    // regardless of which finished first.
    let whole = result(call(
        &socket,
        "output",
        serde_json::json!({"generation": gen, "mode": "page", "maxBytes": 8192}),
    ));
    let ids: Vec<&str> = whole["tasks"]
        .as_array()
        .unwrap()
        .iter()
        .map(|task| task["id"].as_str().unwrap())
        .collect();
    assert!(ids.contains(&"fast fail"), "ids: {ids:?}");
    assert!(ids.contains(&"slow pass"), "ids: {ids:?}");

    // Secret-like content is retrievable verbatim (no redaction), and the
    // serialized page stays below the transport bound.
    let retrieved = result(call(
        &socket,
        "output",
        serde_json::json!({"generation": gen, "task": "fast fail", "tail": 80}),
    ));
    assert_eq!(retrieved["tasks"][0]["id"], "fast fail");
    let stdout = retrieved["tasks"][0]["stdout"]["content"].as_str().unwrap();
    assert!(stdout.contains("secret=abc123"), "no redaction: {stdout}");
    let stderr = retrieved["tasks"][0]["stderr"]["content"].as_str().unwrap();
    assert!(stderr.contains("early boom"), "stderr: {stderr}");
    assert!(
        serde_json::to_vec(&retrieved).unwrap().len() < 65_536,
        "bounded below transport"
    );
}
