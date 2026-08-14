#![cfg(all(feature = "test-integration", unix))]

use serde_json::Value;
use std::io::{BufRead, BufReader, Write};
use std::os::unix::net::UnixStream;
use std::process::{Child, Command, Stdio};
use std::time::{Duration, Instant};

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

#[test]
fn it_runs_a_named_target_over_the_control_socket() {
    let directory = std::env::temp_dir().join(format!("funzzy-control-cli-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&directory);
    std::fs::create_dir_all(&directory).unwrap();
    std::fs::write(
        directory.join(".watch.yaml"),
        r#"
on:
  socket: .tmp/control.sock
tasks:
  - name: fast tests @agent-fast
    run: "true"
    change: "*.txt"
    run_on_init: true
  - name: full tests @agent-final
    run: 'test -z "{{filepath}}"'
    change: ".funzzy-final-never"
"#,
    )
    .unwrap();

    let socket_path = directory.join(".tmp/control.sock");
    let child_log = std::fs::File::create(directory.join("child.err")).unwrap();
    let child = Command::new(env!("CARGO_BIN_EXE_fzz"))
        .current_dir(&directory)
        .env_remove("FUNZZY_BAIL")
        .env_remove("FUNZZY_NON_BLOCK")
        .stdout(Stdio::from(child_log.try_clone().unwrap()))
        .stderr(Stdio::from(child_log))
        .spawn()
        .unwrap();
    let _process = TestProcess { child, directory };

    wait_until(Duration::from_secs(5), || socket_path.exists());
    let targets = call(
        &socket_path,
        serde_json::json!({"jsonrpc": "2.0", "id": "targets", "method": "targets"}),
    );
    assert_eq!(targets["result"][1]["name"], "full tests @agent-final");
    assert_eq!(
        targets["result"][1]["commands"][0],
        "test -z \"{{filepath}}\""
    );

    let run = call(
        &socket_path,
        serde_json::json!({
            "jsonrpc": "2.0",
            "id": "run",
            "method": "run",
            "params": {"target": "@agent-final"}
        }),
    );
    let run_id = run["result"]["runId"].as_u64().unwrap();

    let mut final_status = None;
    wait_until(Duration::from_secs(5), || {
        let status = call(
            &socket_path,
            serde_json::json!({"jsonrpc": "2.0", "id": "status", "method": "status"}),
        );
        let result = &status["result"];
        if result["generation"] == run_id && result["state"] == "passed" {
            final_status = Some(result.clone());
            return true;
        }
        false
    });

    assert_eq!(final_status.unwrap()["trigger"], "control:@agent-final");
}

fn call(path: &std::path::Path, request: Value) -> Value {
    // The control server accepts on a single polling thread (20ms interval);
    // the very first connection can race the accept loop's startup under
    // load and observe an empty response. Retry briefly instead of treating
    // a startup race as a protocol failure.
    let deadline = Instant::now() + Duration::from_secs(5);
    loop {
        let mut stream = UnixStream::connect(path).unwrap();
        writeln!(stream, "{}", request).unwrap();
        let mut response = String::new();
        BufReader::new(stream).read_line(&mut response).unwrap();
        match serde_json::from_str(&response) {
            Ok(parsed) => return parsed,
            Err(err) if Instant::now() < deadline => {
                std::thread::sleep(Duration::from_millis(50));
            }
            Err(err) => panic!(
                "call {:?} -> unparsable response {:?}: {}",
                request.get("method"),
                response,
                err
            ),
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

#[test]
fn emit_routes_a_path_and_returns_matched_with_run_id() {
    let directory = std::env::temp_dir().join(format!("funzzy-emit-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&directory);
    std::fs::create_dir_all(&directory).unwrap();
    std::fs::write(
        directory.join(".watch.yaml"),
        r#"
on:
  socket: .tmp/control.sock
tasks:
  - name: fast tests @agent-fast
    run: "true"
    change: "*.txt"
    run_on_init: true
  - name: full tests @agent-final
    run: 'test -z "{{filepath}}"'
    change: ".funzzy-final-never"
"#,
    )
    .unwrap();

    let socket_path = directory.join(".tmp/control.sock");
    let child_log = std::fs::File::create(directory.join("child.err")).unwrap();
    let child = Command::new(env!("CARGO_BIN_EXE_fzz"))
        .current_dir(&directory)
        .env_remove("FUNZZY_BAIL")
        .env_remove("FUNZZY_NON_BLOCK")
        .stdout(Stdio::from(child_log.try_clone().unwrap()))
        .stderr(Stdio::from(child_log))
        .spawn()
        .unwrap();
    let _process = TestProcess { child, directory };

    wait_until(Duration::from_secs(5), || socket_path.exists());
    let emit = call(
        &socket_path,
        serde_json::json!({
            "jsonrpc": "2.0",
            "id": "emit",
            "method": "emit",
            "params": {"path": "notes.txt"}
        }),
    );
    let result = &emit["result"];
    assert_eq!(result["outcome"], "scheduled");
    assert_eq!(result["matched"][0], "fast tests @agent-fast");
    assert!(result["runId"].is_number(), "runId must be numeric");

    let run_id = result["runId"].as_u64().unwrap();
    wait_until(Duration::from_secs(5), || {
        let status = call(
            &socket_path,
            serde_json::json!({"jsonrpc": "2.0", "id": "status", "method": "status"}),
        );
        status["result"]["generation"] == run_id && status["result"]["state"] == "passed"
    });
}

#[test]
fn emit_unmatched_path_is_explicit_without_generation() {
    let directory =
        std::env::temp_dir().join(format!("funzzy-emit-unmatched-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&directory);
    std::fs::create_dir_all(&directory).unwrap();
    std::fs::write(
        directory.join(".watch.yaml"),
        r#"
on:
  socket: .tmp/control.sock
tasks:
  - name: fast tests @agent-fast
    run: "true"
    change: "*.txt"
    run_on_init: true
"#,
    )
    .unwrap();

    let socket_path = directory.join(".tmp/control.sock");
    let child_log = std::fs::File::create(directory.join("child.err")).unwrap();
    let child = Command::new(env!("CARGO_BIN_EXE_fzz"))
        .current_dir(&directory)
        .env_remove("FUNZZY_BAIL")
        .env_remove("FUNZZY_NON_BLOCK")
        .stdout(Stdio::from(child_log.try_clone().unwrap()))
        .stderr(Stdio::from(child_log))
        .spawn()
        .unwrap();
    let _process = TestProcess { child, directory };

    wait_until(Duration::from_secs(5), || socket_path.exists());
    let emit = call(
        &socket_path,
        serde_json::json!({
            "jsonrpc": "2.0",
            "id": "emit",
            "method": "emit",
            "params": {"path": "src/main.rs"}
        }),
    );
    let result = &emit["result"];
    assert_eq!(result["outcome"], "unmatched");
    assert_eq!(result["matched"], serde_json::json!([]));
    assert!(
        result["runId"].is_null(),
        "no generation for unmatched path"
    );
}

#[test]
fn emit_with_missing_or_empty_path_is_invalid_params() {
    let directory =
        std::env::temp_dir().join(format!("funzzy-emit-invalid-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&directory);
    std::fs::create_dir_all(&directory).unwrap();
    std::fs::write(
        directory.join(".watch.yaml"),
        r#"
on:
  socket: .tmp/control.sock
tasks:
  - name: fast tests @agent-fast
    run: "true"
    change: "*.txt"
    run_on_init: true
"#,
    )
    .unwrap();

    let socket_path = directory.join(".tmp/control.sock");
    let child_log = std::fs::File::create(directory.join("child.err")).unwrap();
    let child = Command::new(env!("CARGO_BIN_EXE_fzz"))
        .current_dir(&directory)
        .env_remove("FUNZZY_BAIL")
        .env_remove("FUNZZY_NON_BLOCK")
        .stdout(Stdio::from(child_log.try_clone().unwrap()))
        .stderr(Stdio::from(child_log))
        .spawn()
        .unwrap();
    let _process = TestProcess { child, directory };

    wait_until(Duration::from_secs(5), || socket_path.exists());

    let missing = call(
        &socket_path,
        serde_json::json!({"jsonrpc": "2.0", "id": "emit-missing", "method": "emit", "params": {}}),
    );
    assert_eq!(missing["error"]["code"], -32602);

    let empty = call(
        &socket_path,
        serde_json::json!({
            "jsonrpc": "2.0",
            "id": "emit-empty",
            "method": "emit",
            "params": {"path": "   "}
        }),
    );
    assert_eq!(empty["error"]["code"], -32602);
}
