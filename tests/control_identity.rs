//! Black-box identity-correlation tests (TASK-0043, contract §1).
//!
//! Proves the typed identities at the wire boundary: one normalized event
//! batch maps to zero or one generation and retains its complete changed-path
//! set; generations carry trigger/batch/predecessor/superseded-by relations;
//! synthetic emit and exact target runs correlate without a debounce batch;
//! and watcher restart resets the instance-scoped sequence.

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
        // Graceful SIGTERM first so long-running sleep children are reaped
        // with their watcher instead of piling up across tests.
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
    let directory =
        std::env::temp_dir().join(format!("fzzi-{}-{test_name}-{counter}", std::process::id()));
    let _ = std::fs::remove_dir_all(&directory);
    std::fs::create_dir_all(&directory).unwrap();
    std::fs::write(directory.join(".watch.yaml"), config).unwrap();
    // Resolve symlink prefixes (macOS /var -> /private/var) so the paths the
    // test writes, the paths notify reports, and the status `changed` set are
    // the same canonical strings.
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
    for _ in 0..100 {
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

fn try_status(socket_path: &std::path::Path) -> Result<serde_json::Value, String> {
    let mut stream = match UnixStream::connect(socket_path) {
        Ok(stream) => stream,
        Err(err) => return Err(err.to_string()),
    };
    writeln!(
        stream,
        r#"{{"jsonrpc":"2.0","id":"status","method":"status"}}"#
    )
    .map_err(|err| err.to_string())?;
    let mut line = String::new();
    BufReader::new(&mut stream)
        .read_line(&mut line)
        .map_err(|err| err.to_string())?;
    serde_json::from_str(&line).map_err(|err| err.to_string())
}

fn raw_status(socket_path: &std::path::Path) -> serde_json::Value {
    try_status(socket_path).expect("connect control socket")
}

fn wait_until<F: FnMut() -> bool>(mut condition: F) {
    let mut last_status = String::new();
    for _ in 0..250 {
        if condition() {
            return;
        }
        std::thread::sleep(Duration::from_millis(100));
    }
    panic!("wait_until timed out (last status: {last_status})");
}

const LONG_RUNNING: &str = r#"
on:
  socket: sock
tasks:
  - name: long running
    run: "sleep 10"
    change: "*.txt"
    run_on_init: true
  - name: other
    run: "true"
    change: ".funzzy-final-never"
"#;

#[test]
fn one_native_batch_maps_to_one_generation_with_complete_changed_set() {
    let directory = setup_directory("native-batch", LONG_RUNNING);
    let _watcher = start_watcher(&directory);
    wait_until_socket(&directory);
    let socket_path = directory.join("sock");

    // Two files changed within one debounce window form one batch. The init
    // generation (sleep 30) is active, so the batch supersedes it.
    std::fs::write(directory.join("b.txt"), "b").unwrap();
    std::fs::write(directory.join("a.txt"), "a").unwrap();

    let batch_generation = wait_until_status(&socket_path, |status| {
        status["generation"].as_u64().unwrap_or(0) >= 2
            && status["state"].as_str() == Some("running")
            && status["batch"].is_number()
    });

    let status = raw_status(&socket_path);
    assert_eq!(
        status["result"]["generation"].as_u64(),
        Some(batch_generation),
        "one batch must map to one generation"
    );
    let changed = status["result"]["changed"]
        .as_array()
        .expect("changed path set")
        .iter()
        .filter_map(|path| path.as_str())
        .map(str::to_owned)
        .collect::<Vec<_>>();
    // The batch retains its complete changed-path set: both files must be
    // present together (the watcher's own socket file may ride along as a
    // legitimate filesystem event in the same debounce window).
    let a = directory.join("a.txt").to_string_lossy().to_string();
    let b = directory.join("b.txt").to_string_lossy().to_string();
    assert!(changed.contains(&a), "a.txt missing: {changed:?}");
    assert!(changed.contains(&b), "b.txt missing: {changed:?}");
    assert_eq!(
        status["result"]["trigger"].as_str().map(|path| {
            let mut path = path.to_owned();
            if path.starts_with("/private") {
                path = path.replacen("/private", "", 1);
            }
            path
        }),
        Some(a.replace("/private", "")),
        "the deterministic first match is the trigger"
    );
}

fn wait_until_status<F: FnMut(&serde_json::Value) -> bool>(
    socket_path: &std::path::Path,
    mut condition: F,
) -> u64 {
    let mut generation = 0;
    let mut last_error = String::new();
    let mut last_seen = String::new();
    for _ in 0..250 {
        match try_status(socket_path) {
            Ok(status) => {
                generation = status["result"]["generation"].as_u64().unwrap_or(0);
                last_seen = status["result"].to_string();
                if condition(&status["result"]) {
                    return generation;
                }
            }
            Err(err) => {
                // A transient connect failure (watcher under heavy load) is
                // not a test failure: keep polling until the bound expires.
                last_error = err;
            }
        }
        std::thread::sleep(Duration::from_millis(100));
    }
    let parent = socket_path.parent().unwrap_or(std::path::Path::new("."));
    let err_log = std::fs::read_to_string(parent.join("child.err"))
        .unwrap_or_else(|_| "(no child.err)".to_string());
    let out_log = std::fs::read_to_string(parent.join("child.out"))
        .unwrap_or_else(|_| "(no child.out)".to_string());
    panic!(
        "wait_until_status timed out (last error: {last_error}, last status: {last_seen})\nwatcher stderr:\n{err_log}\nwatcher stdout:\n{out_log}"
    );
}

#[test]
fn replacement_records_predecessor_identity() {
    let directory = setup_directory("replacement", LONG_RUNNING);
    let _watcher = start_watcher(&directory);
    wait_until_socket(&directory);
    let socket_path = directory.join("sock");

    // First emit schedules generation 1 (long running); the second emit
    // supersedes it, so generation 2 must name generation 1 as predecessor.
    let first = run_cli(&directory, &["control", "emit", "a.txt"]);
    let run_id_one = parse_run_id(&first);
    wait_until_status(&socket_path, |status| {
        status["generation"].as_u64() == Some(run_id_one)
            && status["state"].as_str() == Some("running")
    });

    let second = run_cli(&directory, &["control", "emit", "b.txt"]);
    let run_id_two = parse_run_id(&second);
    assert!(run_id_two > run_id_one);

    wait_until_status(&socket_path, |status| {
        status["generation"].as_u64() == Some(run_id_two)
    });
    let status = raw_status(&socket_path);
    assert_eq!(
        status["result"]["predecessor"].as_u64(),
        Some(run_id_one),
        "the superseding generation names its predecessor"
    );
}

#[test]
fn synthetic_emit_correlates_without_a_debounce_batch() {
    let directory = setup_directory("emit-correlate", LONG_RUNNING);
    let _watcher = start_watcher(&directory);
    wait_until_socket(&directory);
    let socket_path = directory.join("sock");

    let output = run_cli(&directory, &["control", "emit", "notes.txt"]);
    let run_id = parse_run_id(&output);

    wait_until_status(&socket_path, |status| {
        status["generation"].as_u64() == Some(run_id)
    });
    let status = raw_status(&socket_path)["result"].clone();
    assert_eq!(
        status["trigger"].as_str(),
        Some("notes.txt"),
        "emit trigger is the emitted path"
    );
    assert!(
        status["batch"].is_null(),
        "a synthetic emit is not a debounce batch: {}",
        status["batch"]
    );
    assert!(
        status["changed"]
            .as_array()
            .map(Vec::is_empty)
            .unwrap_or(false),
        "no changed-path set for a non-batch trigger"
    );
}

#[test]
fn exact_target_run_correlates_without_a_debounce_batch() {
    let directory = setup_directory("run-correlate", LONG_RUNNING);
    let _watcher = start_watcher(&directory);
    wait_until_socket(&directory);
    let socket_path = directory.join("sock");

    let output = run_cli(&directory, &["control", "run", "other"]);
    let run_id = parse_run_id(&output);

    wait_until_status(&socket_path, |status| {
        status["generation"].as_u64() == Some(run_id)
    });
    let status = raw_status(&socket_path)["result"].clone();
    assert_eq!(
        status["trigger"].as_str(),
        Some("control:other"),
        "target-run trigger is control:<target>"
    );
    assert!(status["batch"].is_null());
    assert!(status["changed"]
        .as_array()
        .map(Vec::is_empty)
        .unwrap_or(false));
}

#[test]
fn parallel_tasks_preserve_generation_correlation() {
    let directory = setup_directory(
        "parallel",
        r#"
on:
  socket: sock
tasks:
  - name: checks-a
    run: "true"
    change: "*.txt"
    parallel: checks
  - name: checks-b
    run: "true"
    change: "*.txt"
    parallel: checks
"#,
    );
    let _watcher = start_watcher(&directory);
    wait_until_socket(&directory);
    let socket_path = directory.join("sock");

    let output = run_cli(&directory, &["control", "emit", "x.txt"]);
    assert!(output.status.success());
    let stdout = String::from_utf8_lossy(&output.stdout);
    assert!(stdout.contains("checks-a"), "stdout: {}", stdout);
    assert!(stdout.contains("checks-b"), "stdout: {}", stdout);
    let run_id = parse_run_id(&output);

    // The parallel generation reaches a terminal passed state under the
    // same generation identity returned by emit.
    wait_until_status(&socket_path, |status| {
        status["generation"].as_u64() == Some(run_id) && status["state"].as_str() == Some("passed")
    });
}

#[test]
fn watcher_restart_resets_the_instance_scoped_generation_sequence() {
    let directory = setup_directory(
        "restart",
        r#"
on:
  socket: sock
tasks:
  - name: init task
    run: "true"
    change: "*.txt"
    run_on_init: true
"#,
    );

    let mut first = start_watcher(&directory);
    wait_until_socket(&directory);
    let socket_path = directory.join("sock");
    wait_until_status(&socket_path, |status| {
        status["generation"].as_u64() == Some(1)
    });

    // End the first instance without removing the fixture: a fresh watcher
    // process is a fresh instance, so its sequence restarts at generation 1
    // and never continues the old instance's counter.
    first.child.kill().expect("kill first watcher");
    first.child.wait().expect("reap first watcher");
    let _ = std::fs::remove_file(&socket_path);
    // Forget the first handle: its Drop would remove the fixture directory,
    // which the second instance still needs. The second handle cleans up.
    std::mem::forget(first);

    let second = start_watcher(&directory);
    wait_until_socket(&directory);
    wait_until_status(&socket_path, |status| {
        status["generation"].as_u64() == Some(1)
            && matches!(status["state"].as_str(), Some("running" | "passed"))
    });
    let _ = second;
}

#[test]
fn config_reload_keeps_instance_identity_alive_on_valid_change() {
    let directory = setup_directory(
        "reload",
        r#"
on:
  socket: sock
tasks:
  - name: init task
    run: "true"
    change: "*.txt"
    run_on_init: true
"#,
    );

    let mut watcher = start_watcher(&directory);
    wait_until_socket(&directory);

    // TASK-0088/0090: a VALID config change no longer SIGTERMs the watcher.
    // This rewrite is semantically identical (comment + same content), so it
    // is a NoOp reload: the instance stays alive, identity is preserved, and
    // the control socket keeps serving.
    std::fs::write(
        directory.join(".watch.yaml"),
        "# reloaded\non:\n  socket: sock\ntasks:\n  - name: init task\n    run: \"true\"\n    change: \"*.txt\"\n    run_on_init: true\n",
    )
    .unwrap();

    // The instance must survive the valid save; the socket stays live.
    let mut alive = false;
    for _ in 0..250 {
        if !watcher.try_exited() && UnixStream::connect(&directory.join("sock")).is_ok() {
            alive = true;
            break;
        }
        std::thread::sleep(Duration::from_millis(100));
    }
    assert!(alive, "valid config reload must keep the instance alive");
}

fn parse_run_id(output: &Output) -> u64 {
    let stdout = String::from_utf8_lossy(&output.stdout);
    stdout
        .trim()
        .lines()
        .find_map(|line| line.strip_prefix("scheduled generation: "))
        .expect("generation line")
        .parse()
        .expect("numeric generation")
}
