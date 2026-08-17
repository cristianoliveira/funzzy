//! TASK-0091 black-box proofs: control identity across a valid config reload.
//!
//! A valid hot reload preserves the watcher instance (token + start time) and
//! the monotonic generation sequence; control `targets`/`run`/`emit` reflect
//! the committed revision with the frozen revision exposed additively; active
//! subscriptions survive the reload and receive the config lifecycle
//! transition; retained output from prior revisions stays retrievable; an
//! invalid candidate publishes the terminal `configInvalid` transition
//! (best effort) before the socket closes and the process exits nonzero; a
//! true process restart still changes the instance token.

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
    fn try_exited(&mut self) -> bool {
        matches!(self.child.try_wait(), Ok(Some(_)))
    }
}

impl Drop for TestProcess {
    fn drop(&mut self) {
        extern "C" {
            fn kill(pid: i32, sig: i32) -> i32;
        }
        unsafe {
            kill(self.child.id() as i32, 15);
        }
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

static DIRECTORY_COUNTER: std::sync::atomic::AtomicU32 = std::sync::atomic::AtomicU32::new(0);

fn setup_directory(test_name: &str, config: &str) -> std::path::PathBuf {
    let counter = DIRECTORY_COUNTER.fetch_add(1, std::sync::atomic::Ordering::SeqCst);
    let directory = std::env::temp_dir().join(format!(
        "fzzri-{}-{test_name}-{counter}",
        std::process::id()
    ));
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
        .env("FUNZZY_COLORED", "false")
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

fn raw_call(socket_path: &std::path::Path, request: &str) -> Result<serde_json::Value, String> {
    let mut stream = UnixStream::connect(socket_path).map_err(|err| err.to_string())?;
    writeln!(stream, "{request}").map_err(|err| err.to_string())?;
    let mut line = String::new();
    BufReader::new(&mut stream)
        .read_line(&mut line)
        .map_err(|err| err.to_string())?;
    serde_json::from_str(&line).map_err(|err| err.to_string())
}

fn status_result(socket_path: &std::path::Path) -> serde_json::Value {
    let response = raw_call(
        socket_path,
        r#"{"jsonrpc":"2.0","id":"status","method":"status","params":{}}"#,
    )
    .expect("status call");
    response["result"].clone()
}

fn capabilities_instance(socket_path: &std::path::Path) -> serde_json::Value {
    let response = raw_call(
        socket_path,
        r#"{"jsonrpc":"2.0","id":"caps","method":"capabilities","params":{}}"#,
    )
    .expect("capabilities call");
    response["result"]["instance"].clone()
}

fn wait_until<F: FnMut() -> bool>(mut condition: F, what: &str) {
    let deadline = std::time::Instant::now() + Duration::from_secs(20);
    while std::time::Instant::now() < deadline {
        if condition() {
            return;
        }
        std::thread::sleep(Duration::from_millis(100));
    }
    panic!("wait_until timed out: {what}");
}

fn wait_for_reload(directory: &std::path::Path, revision: u64) {
    let needle = format!("revision {revision}");
    let deadline = std::time::Instant::now() + Duration::from_secs(20);
    loop {
        if let Ok(log) = std::fs::read_to_string(directory.join("child.err")) {
            if log.contains(&needle) {
                return;
            }
        }
        assert!(
            std::time::Instant::now() < deadline,
            "never observed reload to {needle:?}"
        );
        std::thread::sleep(Duration::from_millis(100));
    }
}

fn base_config() -> String {
    "on:\n  socket: sock\njobs:\n  - name: build\n    run: 'echo build > build-verdict.txt'\n    change: 'src/**'\n"
        .to_owned()
}

fn grown_config() -> String {
    "on:\n  socket: sock\njobs:\n  - name: build\n    run: 'echo build > build-verdict.txt'\n    change: 'src/**'\n  - name: lint\n    run: 'echo lint > lint-verdict.txt'\n    change: '*.md'\n"
        .to_owned()
}

/// AC1: a valid semantic reload preserves the instance token/start time and
/// the generation sequence stays monotonic — never a fresh instance.
#[test]
fn valid_semantic_reload_preserves_instance_and_generation_sequence() {
    let directory = setup_directory("identity", &base_config());
    let mut watcher = start_watcher(&directory);
    wait_until_socket(&directory);
    let socket_path = directory.join("sock");

    let before = capabilities_instance(&socket_path);
    let token_before = before["token"].as_str().expect("token").to_owned();
    let started_before = before["startedAtEpochMs"].as_u64().expect("started");

    // Schedule generation 1 under revision 1.
    let first = run_cli(
        &directory,
        &[
            "control", "--socket", "sock", "--format", "json", "run", "build",
        ],
    );
    assert!(first.status.success(), "first run: {first:?}");
    let first_json: serde_json::Value = serde_json::from_slice(&first.stdout).expect("json");
    let first_generation = first_json["runId"].as_u64().expect("runId");
    assert_eq!(first_json["revision"].as_u64(), Some(1));

    // Valid semantic reload: add the lint job (revision 2).
    std::fs::write(directory.join(".watch.yaml"), grown_config()).unwrap();
    wait_for_reload(&directory, 2);
    assert!(
        !watcher.try_exited(),
        "valid reload must not exit the process"
    );

    // Instance identity is preserved: same token and start time.
    let after = capabilities_instance(&socket_path);
    assert_eq!(
        after["token"].as_str(),
        Some(token_before.as_str()),
        "instance token must survive a valid reload"
    );
    assert_eq!(
        after["startedAtEpochMs"].as_u64(),
        Some(started_before),
        "start time must survive a valid reload"
    );

    // The generation sequence continues monotonically, frozen under the new
    // revision.
    let second = run_cli(
        &directory,
        &[
            "control", "--socket", "sock", "--format", "json", "run", "lint",
        ],
    );
    assert!(second.status.success(), "second run: {second:?}");
    let second_json: serde_json::Value = serde_json::from_slice(&second.stdout).expect("json");
    let second_generation = second_json["runId"].as_u64().expect("runId");
    assert!(
        second_generation > first_generation,
        "generation sequence must stay monotonic: {second_generation} after {first_generation}"
    );
    assert_eq!(
        second_json["revision"].as_u64(),
        Some(2),
        "a generation scheduled after commit freezes the new revision"
    );
}

/// AC6/AC7: control `targets`/`run`/`emit` after commit reflect the new
/// revision consistently, and a stale target is an actionable typed outcome.
#[test]
fn control_surfaces_reflect_committed_revision_and_stale_target_is_typed() {
    let directory = setup_directory("targets", &base_config());
    let _watcher = start_watcher(&directory);
    wait_until_socket(&directory);
    let socket_path = directory.join("sock");

    // Before the reload the stale target does not exist.
    let before_stale = raw_call(
        &socket_path,
        r#"{"jsonrpc":"2.0","id":1,"method":"run","params":{"target":"lint"}}"#,
    )
    .expect("stale run before reload");
    assert_eq!(
        before_stale["error"]["code"],
        serde_json::json!(-32016),
        "a target absent from the current revision is a typed outcome: {before_stale}"
    );
    assert_eq!(before_stale["error"]["message"], "target_not_found");
    assert_eq!(before_stale["error"]["data"]["action"], "reobserve-targets");

    // Reload: add lint (revision 2).
    std::fs::write(directory.join(".watch.yaml"), grown_config()).unwrap();
    wait_for_reload(&directory, 2);

    // `targets` now lists the new job (resolved from the shared config at
    // request time — no server rebuild).
    let targets = raw_call(
        &socket_path,
        r#"{"jsonrpc":"2.0","id":1,"method":"targets","params":{}}"#,
    )
    .expect("targets after reload");
    let names: Vec<String> = targets["result"]
        .as_array()
        .unwrap()
        .iter()
        .filter_map(|target| target["name"].as_str().map(str::to_owned))
        .collect();
    assert!(
        names.contains(&"lint".to_owned()),
        "targets must reflect the committed revision: {names:?}"
    );

    // run of the new job schedules under revision 2.
    let run = raw_call(
        &socket_path,
        r#"{"jsonrpc":"2.0","id":1,"method":"run","params":{"target":"lint"}}"#,
    )
    .expect("run lint after reload");
    assert_eq!(run["result"]["revision"].as_u64(), Some(2));

    // emit binds to the committed revision too.
    let emit = raw_call(
        &socket_path,
        r#"{"jsonrpc":"2.0","id":1,"method":"emit","params":{"path":"src/a.rs"}}"#,
    )
    .expect("emit after reload");
    assert_eq!(emit["result"]["revision"].as_u64(), Some(2));
    assert_eq!(emit["result"]["outcome"], "scheduled");
}

/// AC3/AC4: an active subscription survives a valid reload and receives the
/// config lifecycle transition (`configReloading` → `configReloaded`) on the
/// same connection, with no disconnect/reconnect.
#[test]
fn subscription_survives_reload_and_receives_lifecycle_transition() {
    let directory = setup_directory("subscribe", &base_config());
    let _watcher = start_watcher(&directory);
    wait_until_socket(&directory);
    let socket_path = directory.join("sock");

    let mut stream = UnixStream::connect(&socket_path).expect("connect");
    stream
        .set_read_timeout(Some(Duration::from_secs(10)))
        .expect("read timeout");
    writeln!(
        stream,
        r#"{{"jsonrpc":"2.0","id":"sub","method":"subscribe","params":{{}}}}"#
    )
    .expect("subscribe request");

    // Immediate snapshot (response) plus the first lifecycle transition.
    // ONE persistent reader for the whole connection: a fresh BufReader per
    // read would drop buffered notifications on drop (the server pushes
    // snapshot notifications as fast as reloads publish).
    let mut reader = BufReader::new(&mut stream);
    let mut lines = Vec::new();
    let mut line = String::new();
    reader.read_line(&mut line).expect("immediate snapshot");
    lines.push(line);

    std::fs::write(directory.join(".watch.yaml"), grown_config()).unwrap();
    wait_for_reload(&directory, 2);

    // Read notifications until the reloaded transition arrives; the same
    // connection stays open the whole time (no reconnect).
    let mut saw_reloading = false;
    let mut saw_reloaded = false;
    let deadline = std::time::Instant::now() + Duration::from_secs(10);
    while std::time::Instant::now() < deadline && !(saw_reloading && saw_reloaded) {
        let mut notification = String::new();
        match reader.read_line(&mut notification) {
            Ok(0) => panic!("subscription disconnected mid-reload: {lines:?}"),
            Ok(_) => {
                let value: serde_json::Value = serde_json::from_str(&notification).expect("json");
                lines.push(notification);
                if value["method"] == serde_json::json!("snapshot") {
                    let phase = value["params"]["configLifecycle"]["phase"].as_str();
                    saw_reloading |= phase == Some("configReloading");
                    saw_reloaded |= phase == Some("configReloaded");
                }
            }
            Err(err) => panic!("subscription read error during reload ({err}): {lines:?}"),
        }
    }
    assert!(
        saw_reloaded,
        "subscription must observe the configReloaded transition: {lines:?}"
    );
    assert!(
        saw_reloading,
        "subscription must observe configReloading before configReloaded: {lines:?}"
    );
}

/// AC5: retained output from a prior revision remains retrievable under the
/// same instance after a valid reload (until ordinary eviction).
#[test]
fn retained_output_survives_reload_under_same_instance() {
    let directory = setup_directory(
        "output",
        "on:\n  socket: sock\njobs:\n  - name: failing\n    run: 'echo boom; exit 3'\n    change: 'src/**'\n",
    );
    let _watcher = start_watcher(&directory);
    wait_until_socket(&directory);
    let socket_path = directory.join("sock");

    // Schedule a failing generation under revision 1 and await its evidence.
    let emit = run_cli(
        &directory,
        &[
            "control", "--socket", "sock", "--format", "json", "emit", "src/a.rs",
        ],
    );
    assert!(emit.status.success(), "emit: {emit:?}");
    let emit_json: serde_json::Value = serde_json::from_slice(&emit.stdout).expect("json");
    let generation = emit_json["runId"].as_u64().expect("runId");
    assert_eq!(emit_json["revision"].as_u64(), Some(1));

    wait_until(
        || {
            status_result(&socket_path)["generation"].as_u64() == Some(generation)
                && status_result(&socket_path)["state"].as_str() == Some("failed")
        },
        "generation to fail",
    );

    // Output is retrievable under the instance token.
    let token = capabilities_instance(&socket_path)["token"]
        .as_str()
        .expect("token")
        .to_owned();
    let output_before = raw_call(
        &socket_path,
        &format!(
            r#"{{"jsonrpc":"2.0","id":1,"method":"output","params":{{"generation":{generation},"instanceToken":"{token}"}}}}"#
        ),
    )
    .expect("output before reload");
    assert!(output_before["result"]["tasks"][0]["stdout"]["content"]
        .as_str()
        .unwrap()
        .contains("boom"));
    assert_eq!(
        output_before["result"]["revision"].as_u64(),
        Some(1),
        "output carries the frozen revision of the generation"
    );

    // Valid reload to revision 2.
    std::fs::write(directory.join(".watch.yaml"), grown_config()).unwrap();
    wait_for_reload(&directory, 2);

    // The prior revision's evidence is still retrievable under the SAME
    // instance token — no restart reset the registry.
    let output_after = raw_call(
        &socket_path,
        &format!(
            r#"{{"jsonrpc":"2.0","id":1,"method":"output","params":{{"generation":{generation},"instanceToken":"{token}"}}}}"#
        ),
    )
    .expect("output after reload");
    assert_eq!(
        output_after["result"]["revision"].as_u64(),
        Some(1),
        "prior-revision evidence keeps its frozen revision"
    );
    assert!(output_after["result"]["tasks"][0]["stdout"]["content"]
        .as_str()
        .unwrap()
        .contains("boom"));
}

/// AC8: an invalid candidate publishes the terminal `configInvalid` lifecycle
/// transition (best effort) before the socket closes; the process exits
/// nonzero with the terminal error visible.
#[test]
fn invalid_config_publishes_terminal_lifecycle_and_exits_nonzero() {
    let directory = setup_directory("invalid", &base_config());
    let mut watcher = start_watcher(&directory);
    wait_until_socket(&directory);
    let socket_path = directory.join("sock");

    // Subscribe so the terminal transition can reach a client.
    let mut stream = UnixStream::connect(&socket_path).expect("connect");
    stream
        .set_read_timeout(Some(Duration::from_millis(200)))
        .expect("read timeout");
    writeln!(
        stream,
        r#"{{"jsonrpc":"2.0","id":"sub","method":"subscribe","params":{{}}}}"#
    )
    .expect("subscribe");

    std::fs::write(directory.join(".watch.yaml"), "jobs: [unclosed").unwrap();

    // The watcher must exit nonzero with the terminal error.
    let deadline = std::time::Instant::now() + Duration::from_secs(20);
    let exit = loop {
        if let Some(status) = watcher.child.try_wait().expect("try_wait") {
            break status;
        }
        assert!(
            std::time::Instant::now() < deadline,
            "invalid config must terminate the watcher"
        );
        std::thread::sleep(Duration::from_millis(50));
    };
    assert!(!exit.success(), "invalid config exits nonzero: {exit:?}");

    let log = std::fs::read_to_string(directory.join("child.err")).unwrap_or_default();
    assert!(
        log.contains("Fatal configuration error"),
        "terminal error must be visible: {log}"
    );

    // Best effort: whatever the subscriber observed before disconnect was
    // the terminal transition, never a generic snapshot of a live watcher.
    // The socket closes after the terminal event attempt (bounded read).
    let mut observed = String::new();
    let mut buffer = String::new();
    loop {
        match BufReader::new(&mut stream).read_line(&mut buffer) {
            Ok(0) | Err(_) => break,
            Ok(_) => {
                observed.push_str(&buffer);
                buffer.clear();
            }
        }
    }
    if !observed.is_empty() {
        let value: serde_json::Value = serde_json::from_str(&observed).expect("json");
        if value["method"] == serde_json::json!("snapshot") {
            let phase = value["params"]["configLifecycle"]["phase"].as_str();
            assert_eq!(
                phase,
                Some("configInvalid"),
                "terminal transition before disconnect: {observed}"
            );
            assert!(
                value["params"]["configLifecycle"]["reason"]
                    .as_str()
                    .is_some(),
                "configInvalid carries the gate/reason: {observed}"
            );
        }
    }
}

/// AC9: a true process restart still changes the instance token; the config
/// revision never weakens restart freshness.
#[test]
fn restart_changes_instance_token_while_reload_does_not() {
    let directory = setup_directory(
        "restart",
        "on:\n  socket: sock\njobs:\n  - name: build\n    run: 'echo build > build-verdict.txt'\n    change: 'src/**'\n",
    );

    let mut first = start_watcher(&directory);
    wait_until_socket(&directory);
    let socket_path = directory.join("sock");
    let first_token = capabilities_instance(&socket_path)["token"]
        .as_str()
        .expect("token")
        .to_owned();

    // A valid reload preserves the token (AC1), then a true restart changes it.
    std::fs::write(directory.join(".watch.yaml"), grown_config()).unwrap();
    wait_for_reload(&directory, 2);
    let after_reload = capabilities_instance(&socket_path)["token"]
        .as_str()
        .expect("token")
        .to_owned();
    assert_eq!(
        after_reload, first_token,
        "reload must not change the token"
    );

    first.child.kill().expect("kill first watcher");
    first.child.wait().expect("reap first watcher");
    let _ = std::fs::remove_file(&socket_path);
    std::mem::forget(first);

    let second = start_watcher(&directory);
    wait_until_socket(&directory);
    let second_token = capabilities_instance(&socket_path)["token"]
        .as_str()
        .expect("token")
        .to_owned();
    assert_ne!(
        second_token, first_token,
        "restart must change the instance token"
    );
    let _ = second;
}
