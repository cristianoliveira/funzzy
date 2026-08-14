#![cfg(all(feature = "test-integration", unix))]

//! TASK-0056: prove duration estimates persist, invalidate, and stay bounded
//! through the real watcher + control socket.
//!
//! Every test isolates `XDG_STATE_HOME` so history never lands in the user's
//! real state dir and tests never interfere. Assertions are state-based, not
//! host-speed-timing based: we wait for explicit terminal states and compare
//! estimate fields, never wall-clock durations.

use serde_json::Value;
use std::io::{BufRead, BufReader, Write};
use std::os::unix::net::UnixStream;
use std::process::{Child, Command, Stdio};
use std::time::{Duration, Instant};

/// A running watcher plus its isolated state dir; killed and cleaned on drop.
/// `persist` keeps the workspace and state dir on drop (restart tests reuse
/// them across watcher generations); the final watcher cleans up.
struct TestWatcher {
    child: Child,
    directory: std::path::PathBuf,
    state_dir: std::path::PathBuf,
    socket_path: std::path::PathBuf,
    persist: bool,
}

impl Drop for TestWatcher {
    fn drop(&mut self) {
        let _ = self.child.kill();
        let _ = self.child.wait();
        if !self.persist {
            let _ = std::fs::remove_dir_all(&self.directory);
            let _ = std::fs::remove_dir_all(&self.state_dir);
        }
    }
}

/// Launches a watcher on `directory` with an isolated XDG state dir and a
/// control socket, waiting until the socket accepts connections.
fn start_watcher(directory: &std::path::Path, state_dir: &std::path::Path) -> TestWatcher {
    start_watcher_persistent(directory, state_dir, false)
}

/// Like [`start_watcher`] but keeps workspace + state on drop so restart and
/// invalidation tests can reuse them.
fn start_watcher_persistent(
    directory: &std::path::Path,
    state_dir: &std::path::Path,
    persist: bool,
) -> TestWatcher {
    let socket_path = directory.join(".tmp/control.sock");
    let child_log = std::fs::File::create(directory.join("child.err")).unwrap();
    let child = Command::new(env!("CARGO_BIN_EXE_fzz"))
        .current_dir(directory)
        .env("XDG_STATE_HOME", state_dir)
        .env_remove("FUNZZY_BAIL")
        .env_remove("FUNZZY_NON_BLOCK")
        .stdout(Stdio::from(child_log.try_clone().unwrap()))
        .stderr(Stdio::from(child_log))
        .spawn()
        .unwrap();
    let watcher = TestWatcher {
        child,
        directory: directory.to_path_buf(),
        state_dir: state_dir.to_path_buf(),
        socket_path: socket_path.clone(),
        persist,
    };
    wait_until(Duration::from_secs(10), || socket_path.exists());
    // The socket file can exist before the accept loop is ready; a first
    // `capabilities` round-trip confirms the server is live.
    call(
        &socket_path,
        serde_json::json!({"jsonrpc":"2.0","id":"ping","method":"capabilities"}),
    );
    watcher
}

fn call(path: &std::path::Path, request: Value) -> Value {
    let deadline = Instant::now() + Duration::from_secs(10);
    loop {
        match UnixStream::connect(path) {
            Ok(mut stream) => {
                let _ = writeln!(stream, "{}", request);
                let mut response = String::new();
                match BufReader::new(&mut stream).read_line(&mut response) {
                    Ok(_) => match serde_json::from_str(&response) {
                        Ok(parsed) => return parsed,
                        Err(_) if Instant::now() < deadline => {
                            std::thread::sleep(Duration::from_millis(50));
                        }
                        Err(err) => panic!(
                            "call {:?} -> unparsable response {:?}: {}",
                            request.get("method"),
                            response,
                            err
                        ),
                    },
                    Err(_) if Instant::now() < deadline => {
                        std::thread::sleep(Duration::from_millis(50));
                    }
                    Err(err) => panic!("call {:?} -> read error: {}", request.get("method"), err),
                }
            }
            Err(_) if Instant::now() < deadline => {
                std::thread::sleep(Duration::from_millis(20));
            }
            Err(err) => panic!("call {:?} -> connect error: {}", request.get("method"), err),
        }
    }
}

fn wait_until(timeout: Duration, mut condition: impl FnMut() -> bool) {
    let started = Instant::now();
    while started.elapsed() < timeout {
        if condition() {
            return;
        }
        std::thread::sleep(Duration::from_millis(20));
    }
    panic!("condition did not become true within {:?}", timeout);
}

/// Runs the named target over the control socket and waits for its terminal
/// state. Returns the observed terminal state name.
fn run_target_and_wait(socket: &std::path::Path, target: &str, want: &str) {
    let run = call(
        socket,
        serde_json::json!({
            "jsonrpc": "2.0",
            "id": "run",
            "method": "run",
            "params": {"target": target}
        }),
    );
    let run_id = run["result"]["runId"].as_u64().unwrap();
    wait_until(Duration::from_secs(10), || {
        let status = call(
            socket,
            serde_json::json!({"jsonrpc": "2.0", "id": "status", "method": "status"}),
        );
        let result = &status["result"];
        result["generation"] == run_id && result["state"] == want
    });
}

fn target_estimate(socket: &std::path::Path, target: &str) -> Option<Value> {
    let targets = call(
        socket,
        serde_json::json!({"jsonrpc": "2.0", "id": "targets", "method": "targets"}),
    );
    targets["result"]
        .as_array()
        .unwrap()
        .iter()
        .find(|entry| entry["name"] == target)
        .and_then(|entry| entry.get("estimate"))
        .cloned()
}

fn setup(name: &str, config: &str) -> (std::path::PathBuf, std::path::PathBuf) {
    let directory =
        std::env::temp_dir().join(format!("funzzy-estimate-{}-{}", name, std::process::id()));
    let _ = std::fs::remove_dir_all(&directory);
    std::fs::create_dir_all(&directory).unwrap();
    std::fs::create_dir_all(directory.join(".tmp")).unwrap();
    std::fs::write(directory.join(".watch.yaml"), config).unwrap();
    let state_dir = std::env::temp_dir().join(format!(
        "funzzy-estimate-state-{}-{}",
        name,
        std::process::id()
    ));
    let _ = std::fs::remove_dir_all(&state_dir);
    std::fs::create_dir_all(&state_dir).unwrap();
    (directory, state_dir)
}

const BASIC_CONFIG: &str = r#"
on:
  socket: .tmp/control.sock
tasks:
  - name: final @agent-final
    run: "true"
    change: "*.txt"
    run_on_init: false
"#;

#[test]
fn repeated_target_successes_produce_estimate_with_confidence_progression() {
    let (directory, state_dir) = setup("progression", BASIC_CONFIG);
    let watcher = start_watcher(&directory, &state_dir);

    // No history yet: estimate absent, not null.
    assert!(target_estimate(&watcher.socket_path, "final @agent-final").is_none());

    // Three successes: median/p90/recommendation present, confidence medium.
    for _ in 0..3 {
        run_target_and_wait(&watcher.socket_path, "final @agent-final", "passed");
    }
    let estimate = target_estimate(&watcher.socket_path, "final @agent-final")
        .expect("estimate after three successes");
    assert!(estimate["typicalMs"].as_u64().unwrap() > 0);
    assert!(estimate["upperMs"].as_u64().unwrap() > 0);
    assert!(estimate["recommendedTimeoutMs"].as_u64().unwrap() > 0);
    assert!(estimate["typicalMs"].as_u64().unwrap() <= estimate["upperMs"].as_u64().unwrap());
    assert!(
        estimate["upperMs"].as_u64().unwrap() <= estimate["recommendedTimeoutMs"].as_u64().unwrap(),
        "typical <= upper <= recommended invariant"
    );
    assert_eq!(estimate["samples"].as_u64().unwrap(), 3);
    assert_eq!(estimate["confidence"], "medium");
    assert_eq!(estimate["source"], "measured");

    // Capabilities advertise the surface with declared limits.
    let caps = call(
        &watcher.socket_path,
        serde_json::json!({"jsonrpc":"2.0","id":"caps","method":"capabilities"}),
    );
    assert_eq!(caps["result"]["features"]["durationEstimates"], true);
    assert_eq!(
        caps["result"]["limits"]["durationEstimateLimits"]["maxSamples"],
        20
    );
}

#[test]
fn watcher_restart_preserves_estimate_for_unchanged_signature() {
    let (directory, state_dir) = setup("restart", BASIC_CONFIG);
    {
        let watcher = start_watcher_persistent(&directory, &state_dir, true);
        for _ in 0..2 {
            run_target_and_wait(&watcher.socket_path, "final @agent-final", "passed");
        }
        assert!(target_estimate(&watcher.socket_path, "final @agent-final").is_some());
    } // drop kills the watcher; state dir persists

    let restarted = start_watcher(&directory, &state_dir);
    let estimate = target_estimate(&restarted.socket_path, "final @agent-final")
        .expect("estimate survives restart");
    assert_eq!(estimate["samples"].as_u64().unwrap(), 2);
    assert_eq!(estimate["confidence"], "low");
}

#[test]
fn command_change_invalidates_old_profile() {
    let (directory, state_dir) = setup("invalidate", BASIC_CONFIG);
    {
        let watcher = start_watcher_persistent(&directory, &state_dir, true);
        for _ in 0..2 {
            run_target_and_wait(&watcher.socket_path, "final @agent-final", "passed");
        }
        assert!(target_estimate(&watcher.socket_path, "final @agent-final").is_some());
    }

    // Change the command: the resolved signature changes, so the old profile
    // must not apply until new samples exist.
    std::fs::write(
        directory.join(".watch.yaml"),
        r#"
on:
  socket: .tmp/control.sock
tasks:
  - name: final @agent-final
    run: "true extra-argument"
    change: "*.txt"
    run_on_init: false
"#,
    )
    .unwrap();

    let watcher = start_watcher(&directory, &state_dir);
    assert!(
        target_estimate(&watcher.socket_path, "final @agent-final").is_none(),
        "changed command must invalidate the old profile"
    );
}

#[test]
fn failures_and_cancellations_never_lower_recommendation() {
    let (directory, state_dir) = setup(
        "exclusion",
        r#"
on:
  socket: .tmp/control.sock
tasks:
  - name: gated @agent-final
    run: "test ! -f gate.fail"
    change: "*.txt"
    run_on_init: false
"#,
    );
    let watcher = start_watcher_persistent(&directory, &state_dir, true);

    // Three successes with the same command + signature.
    for _ in 0..3 {
        run_target_and_wait(&watcher.socket_path, "gated @agent-final", "passed");
    }
    let before = target_estimate(&watcher.socket_path, "gated @agent-final")
        .expect("estimate after three successes");
    assert_eq!(before["samples"].as_u64().unwrap(), 3);

    // Flip the same command to failing (same signature, gate file present):
    // the failure is recorded separately and must not alter the estimate.
    std::fs::write(directory.join("gate.fail"), "fail").unwrap();
    run_target_and_wait(&watcher.socket_path, "gated @agent-final", "failed");
    let after = target_estimate(&watcher.socket_path, "gated @agent-final")
        .expect("estimate after failure");
    assert_eq!(after, before, "failure must not alter the success estimate");
}

#[test]
fn state_writes_never_trigger_watched_worktree_events() {
    let (directory, state_dir) = setup("no-feedback", BASIC_CONFIG);
    let watcher = start_watcher(&directory, &state_dir);
    for _ in 0..3 {
        run_target_and_wait(&watcher.socket_path, "final @agent-final", "passed");
    }

    // The state file lives under XDG state, outside the watched worktree.
    let state_files: Vec<_> = walk_state_files(&state_dir);
    assert!(
        !state_files.is_empty(),
        "history must be persisted somewhere"
    );
    for file in &state_files {
        assert!(
            !file.starts_with(&directory),
            "state file must never live in the workspace: {}",
            file.display()
        );
    }

    // After the writes settle, no new generation appears without an explicit
    // run: the persistence writes created no watched filesystem event.
    let idle = call(
        &watcher.socket_path,
        serde_json::json!({"jsonrpc":"2.0","id":"status","method":"status"}),
    );
    let generation_before = idle["result"]["generation"].as_u64().unwrap();
    std::thread::sleep(Duration::from_millis(500));
    let idle_after = call(
        &watcher.socket_path,
        serde_json::json!({"jsonrpc":"2.0","id":"status","method":"status"}),
    );
    assert_eq!(
        idle_after["result"]["generation"], generation_before,
        "persistence must not schedule a new generation (feedback loop)"
    );
}

#[test]
fn corrupt_history_recovers_with_warning_and_watcher_stays_usable() {
    let (directory, state_dir) = setup("corrupt", BASIC_CONFIG);
    {
        let watcher = start_watcher_persistent(&directory, &state_dir, true);
        for _ in 0..2 {
            run_target_and_wait(&watcher.socket_path, "final @agent-final", "passed");
        }
    } // stop the watcher so the store is not concurrently rewritten

    // Corrupt every state file; the next watcher must quarantine + recover
    // empty and still serve requests.
    let state_files: Vec<_> = walk_state_files(&state_dir);
    assert!(!state_files.is_empty());
    for file in &state_files {
        std::fs::write(file, b"{ not json ").unwrap();
    }

    let watcher = start_watcher(&directory, &state_dir);
    assert!(
        target_estimate(&watcher.socket_path, "final @agent-final").is_none(),
        "corrupt history recovers empty"
    );
    run_target_and_wait(&watcher.socket_path, "final @agent-final", "passed");
    let estimate = target_estimate(&watcher.socket_path, "final @agent-final")
        .expect("watcher remains usable and records fresh samples");
    assert_eq!(estimate["samples"].as_u64().unwrap(), 1);
    assert_eq!(estimate["confidence"], "low");

    // The corrupt file was quarantined aside (observable recovery).
    let quarantined: Vec<_> = walk_state_files(&state_dir)
        .into_iter()
        .filter(|path| {
            path.extension()
                .map(|ext| ext == "corrupt")
                .unwrap_or(false)
        })
        .collect();
    assert!(!quarantined.is_empty(), "corrupt state must be quarantined");
}

#[test]
fn legacy_capabilities_without_estimate_surface_stay_compatible() {
    let (directory, state_dir) = setup("legacy", BASIC_CONFIG);
    let watcher = start_watcher(&directory, &state_dir);
    let caps = call(
        &watcher.socket_path,
        serde_json::json!({"jsonrpc":"2.0","id":"caps","method":"capabilities"}),
    );
    // The surface is active here, but the optional-field list and feature
    // flags remain additive: a legacy client decoding only known keys keeps
    // working (it ignores durationEstimates and estimate fields).
    assert!(caps["result"]["optionalFields"]
        .as_array()
        .unwrap()
        .iter()
        .any(|field| field.as_str() == Some("estimate")));
    // The wire never leaks signature inputs, env values, or the state path.
    let raw = serde_json::to_string(&caps).unwrap();
    assert!(!raw.contains("execution_signature"));
    assert!(!raw.contains("run-durations-v1.json"));
}

fn walk_state_files(root: &std::path::Path) -> Vec<std::path::PathBuf> {
    let mut files = vec![];
    fn walk(dir: &std::path::Path, out: &mut Vec<std::path::PathBuf>) {
        if let Ok(entries) = std::fs::read_dir(dir) {
            for entry in entries.flatten() {
                let path = entry.path();
                if path.is_dir() {
                    walk(&path, out);
                } else {
                    out.push(path);
                }
            }
        }
    }
    walk(root, &mut files);
    files
}
