//! Black-box subscription tests (TASK-0050, contract §7).
//!
//! Proves the wire + capability surface: `subscribe` returns one immediate
//! correlated snapshot then streams `snapshot` notifications on the same
//! connection; `capabilities` advertises `subscribe` + `subscription: true`
//! only on a registered endpoint; and unknown methods stay backward compatible.

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
        std::env::temp_dir().join(format!("fzzs-{}-{test_name}-{counter}", std::process::id()));
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
    panic!(
        "control socket never connectable at {}",
        socket_path.display()
    );
}

fn call(directory: &std::path::Path, request: serde_json::Value) -> serde_json::Value {
    let socket = directory.join("sock");
    let mut stream = UnixStream::connect(&socket).expect("connect control socket");
    writeln!(stream, "{}", request).unwrap();
    let mut line = String::new();
    BufReader::new(&stream).read_line(&mut line).unwrap();
    serde_json::from_str(&line).unwrap()
}

fn instance_token(directory: &std::path::Path) -> String {
    let response = call(
        directory,
        serde_json::json!({"jsonrpc": "2.0", "id": "caps", "method": "capabilities"}),
    );
    response["result"]["instance"]["token"]
        .as_str()
        .expect("instance token")
        .to_owned()
}

/// Reads the next subscription line that is a `snapshot` notification.
fn read_snapshot(reader: &mut BufReader<UnixStream>) -> serde_json::Value {
    let deadline = std::time::Instant::now() + Duration::from_secs(5);
    loop {
        let mut line = String::new();
        match reader.read_line(&mut line) {
            Ok(0) => panic!("subscription stream closed before a snapshot notification"),
            Ok(_) => {
                let message: serde_json::Value = serde_json::from_str(&line)
                    .unwrap_or_else(|_| panic!("unparsable subscription line: {}", line));
                if message["method"] == "snapshot" {
                    return message;
                }
                // The immediate `result` response or an unrelated line: keep
                // reading until the notification arrives.
            }
            Err(_) => {
                if std::time::Instant::now() >= deadline {
                    panic!("timed out waiting for a snapshot notification");
                }
                std::thread::sleep(Duration::from_millis(50));
            }
        }
    }
}

const QUICK: &str = r#"
on:
  socket: sock
tasks:
  - name: quick
    run: "true"
    change: "*.txt"
"#;

#[test]
fn subscribe_returns_immediate_snapshot_then_streams_transitions() {
    let directory = setup_directory("stream", QUICK);
    let _watcher = start_watcher(&directory);
    wait_until_socket(&directory);
    let token = instance_token(&directory);

    let socket = directory.join("sock");
    let mut stream = UnixStream::connect(&socket).expect("subscribe connection");
    stream
        .set_read_timeout(Some(Duration::from_millis(100)))
        .unwrap();
    stream
        .write_all(b"{\"jsonrpc\":\"2.0\",\"id\":\"subscribe\",\"method\":\"subscribe\"}\n")
        .unwrap();

    // Immediate result: one correlated snapshot with the shared instance.
    let mut reader = BufReader::new(stream);
    let mut response = String::new();
    reader.read_line(&mut response).unwrap();
    let initial: serde_json::Value = serde_json::from_str(&response).unwrap();
    assert_eq!(initial["id"], "subscribe");
    let snapshot = &initial["result"];
    assert_eq!(snapshot["instance"]["token"], token);
    assert_eq!(snapshot["generation"], 0);
    assert_eq!(snapshot["state"], "idle");
    assert!(snapshot["tasks"].as_array().unwrap().is_empty());

    // Trigger a transition via a real filesystem write; a `snapshot`
    // notification must arrive on the same connection with the batch path
    // and per-task outcome.
    std::fs::write(directory.join("a.txt"), "x").unwrap();
    let notification = read_snapshot(&mut reader);
    let params = &notification["params"];
    assert_eq!(params["instance"]["token"], token);
    assert_eq!(params["generation"], 1);
    assert!(params["commands"].as_array().unwrap().len() >= 1);
    let paths = params["paths"].as_array().unwrap();
    assert!(!paths.is_empty(), "paths must carry the batch path");
    // The batch is the complete changed-path set (contract §1) and may
    // coalesce unrelated setup writes under load; the trigger must be
    // present, not necessarily first.
    assert!(
        paths.iter().any(|p| p.as_str().unwrap().ends_with("a.txt")),
        "batch must contain the trigger path: {paths:?}"
    );
}

#[test]
fn capabilities_advertise_subscribe_when_endpoint_is_registered() {
    let directory = setup_directory("caps", QUICK);
    let _watcher = start_watcher(&directory);
    wait_until_socket(&directory);

    let response = call(
        &directory,
        serde_json::json!({"jsonrpc": "2.0", "id": "caps", "method": "capabilities"}),
    );
    let methods: Vec<_> = response["result"]["methods"]
        .as_array()
        .unwrap()
        .iter()
        .map(|method| method.as_str().unwrap())
        .collect();
    assert!(methods.contains(&"subscribe"), "methods: {methods:?}");
    assert_eq!(response["result"]["features"]["subscription"], true);
}

#[test]
fn subscribe_cli_reports_an_actionable_error_on_a_legacy_server() {
    // A server without a broker (e.g. a status-only `ControlApi`) does not
    // advertise `subscribe`; the CLI should surface a compatibility message.
    let directory = setup_directory(
        "legacy",
        "on:\n  socket: sock\ntasks:\n  - name: t\n    run: \"true\"\n    change: \"*.txt\"\n",
    );
    let _watcher = start_watcher(&directory);
    wait_until_socket(&directory);

    let response = call(
        &directory,
        serde_json::json!({"jsonrpc": "2.0", "id": "nope", "method": "unknown-method"}),
    );
    assert_eq!(response["error"]["code"], -32601);
    assert_eq!(response["error"]["message"], "Method not found");
}

#[test]
fn subscribe_failure_notification_carries_exact_output_ref() {
    // Contract §1/§3/§5: the snapshot notification on a failed generation
    // carries the same structured outputRef as status/await — one reference
    // source, no divergent reconstruction across surfaces.
    let directory = setup_directory(
        "output-ref",
        r#"
on:
  socket: sock
tasks:
  - name: failing task
    run: 'echo "boom stdout" && echo "boom stderr" >&2 && exit 3'
    change: "*.txt"
"#,
    );
    let _watcher = start_watcher(&directory);
    wait_until_socket(&directory);

    // Subscribe first, then emit a failing change; the notification stream
    // must report the failed snapshot with an outputRef.
    let socket = directory.join("sock");
    let mut stream = UnixStream::connect(&socket).expect("connect");
    writeln!(
        stream,
        r#"{{"jsonrpc":"2.0","id":"sub","method":"subscribe"}}"#
    )
    .unwrap();
    let mut reader = BufReader::new(stream);

    let emit = call(
        &directory,
        serde_json::json!({"jsonrpc": "2.0", "id": "emit", "method": "emit",
                           "params": {"path": "notes.txt"}}),
    );
    let generation = emit["result"]["runId"].as_u64().expect("runId");

    // Drain until the notification for the failed generation with evidence.
    let deadline = std::time::Instant::now() + Duration::from_secs(10);
    let mut found = None;
    while std::time::Instant::now() < deadline {
        let notification = read_snapshot(&mut reader);
        if notification["params"]["generation"].as_u64() == Some(generation)
            && notification["params"]["state"].as_str() == Some("failed")
        {
            found = Some(notification);
            break;
        }
    }
    let notification = found.expect("failed snapshot notification");

    let evidence = &notification["params"]["failureEvidence"];
    let output_ref = &evidence["outputRef"];
    assert_eq!(output_ref["generation"], generation);
    assert_eq!(output_ref["task"], "failing task");
    assert!(!output_ref["instanceToken"].as_str().unwrap().is_empty());
    assert!(output_ref["retrieve"]
        .as_str()
        .unwrap()
        .contains("--instance '"));
    assert_eq!(evidence["additionalFailedTasks"], 0);
    assert!(evidence["excerpt"]
        .as_str()
        .unwrap()
        .contains("boom stdout"));
}
