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
    let child = Command::new(env!("CARGO_BIN_EXE_fzz"))
        .current_dir(&directory)
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .spawn()
        .unwrap();
    let _process = TestProcess { child, directory };

    wait_until(Duration::from_secs(3), || socket_path.exists());
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
    wait_until(Duration::from_secs(3), || {
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
    let mut stream = UnixStream::connect(path).unwrap();
    writeln!(stream, "{}", request).unwrap();
    let mut response = String::new();
    BufReader::new(stream).read_line(&mut response).unwrap();
    serde_json::from_str(&response).unwrap()
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
