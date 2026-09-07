//! Black-box atomic-await tests (TASK-0044, contract §4/§3).
//!
//! Proves the wire + CLI surface: unambiguous modes, already-terminal and
//! future completion, no-generation-yet, superseded during wait, watcher
//! disconnect/restart, multiple waiters, timeout boundary, `run/emit --wait`
//! composition, and freshness classification in the returned observation.

#![cfg(all(feature = "test-integration", unix))]

use std::io::{BufRead, BufReader, Write};
use std::os::unix::fs::PermissionsExt;
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
        // Graceful SIGTERM first: the watcher's shutdown handler reaps its
        // task process groups, so long-running sleep children never pile up
        // across tests. SIGKILL is the fallback for a stuck watcher.
        let _ = libc_kill(self.child.id() as i32, 15);
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

/// Minimal libc kill wrapper (signal 15 = SIGTERM) without extra deps.
fn libc_kill(pid: i32, signal: i32) -> std::io::Result<()> {
    let result = unsafe { libc_kill_impl(pid, signal) };
    if result == 0 {
        Ok(())
    } else {
        Err(std::io::Error::last_os_error())
    }
}

unsafe fn libc_kill_impl(pid: i32, signal: i32) -> i32 {
    // FFI to kill(2) via the standard library's process primitives.
    extern "C" {
        fn kill(pid: i32, sig: i32) -> i32;
    }
    kill(pid, signal)
}

static DIRECTORY_COUNTER: std::sync::atomic::AtomicU32 = std::sync::atomic::AtomicU32::new(0);

fn setup_directory(test_name: &str, config: &str) -> std::path::PathBuf {
    let counter = DIRECTORY_COUNTER.fetch_add(1, std::sync::atomic::Ordering::SeqCst);
    let directory =
        std::env::temp_dir().join(format!("fzza-{}-{test_name}-{counter}", std::process::id()));
    let _ = std::fs::remove_dir_all(&directory);
    std::fs::create_dir_all(&directory).unwrap();
    std::fs::write(directory.join(".watch.yaml"), config).unwrap();
    std::fs::canonicalize(&directory).expect("canonicalize fixture root")
}

fn start_watcher(directory: &std::path::Path) -> TestProcess {
    start_watcher_with_env(directory, &[])
}

/// Spawns the watcher with extra environment pairs; used to shrink the
/// deterministic cancel/shutdown grace for escalation proofs.
fn start_watcher_with_env(directory: &std::path::Path, env: &[(&str, &str)]) -> TestProcess {
    let child_log = std::fs::File::create(directory.join("child.err")).unwrap();
    let mut command = Command::new(env!("CARGO_BIN_EXE_fzz"));
    command
        .current_dir(directory)
        // Isolate from the ambient environment: fail-fast and non-block flags
        // must come from the test's own config, not the developer's shell.
        .env_remove("FUNZZY_BAIL")
        .env_remove("FUNZZY_NON_BLOCK");
    for (key, value) in env {
        command.env(key, value);
    }
    let child = command
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

fn spawn_cli(directory: &std::path::Path, args: &[&str]) -> Child {
    Command::new(env!("CARGO_BIN_EXE_fzz"))
        .current_dir(directory)
        .args(args)
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .expect("fzz control client should spawn")
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

fn run_cli_retry(directory: &std::path::Path, args: &[&str]) -> Output {
    let mut last = run_cli(directory, args);
    for _ in 0..2 {
        if last.status.success() {
            return last;
        }
        std::thread::sleep(Duration::from_millis(500));
        last = run_cli(directory, args);
    }
    last
}

fn wait_until_status<F: FnMut(&serde_json::Value) -> bool>(socket: &std::path::Path, mut f: F) {
    let mut last_error = String::new();
    let mut last_seen = String::new();
    for _ in 0..300 {
        match try_status(socket) {
            Ok(status) => {
                last_seen = status["result"].to_string();
                if f(status.get("result").unwrap_or(&serde_json::Value::Null)) {
                    return;
                }
            }
            // A transient connect failure (watcher under heavy load) is not a
            // test failure: keep polling until the bound expires.
            Err(err) => last_error = err,
        }
        std::thread::sleep(Duration::from_millis(100));
    }
    let parent = socket.parent().unwrap_or(std::path::Path::new("."));
    let err_log = std::fs::read_to_string(parent.join("child.err"))
        .unwrap_or_else(|_| "(no child.err)".to_string());
    let out_log = std::fs::read_to_string(parent.join("child.out"))
        .unwrap_or_else(|_| "(no child.out)".to_string());
    panic!(
        "wait_until_status timed out (last error: {last_error}, last status: {last_seen})\nwatcher stderr:\n{err_log}\nwatcher stdout:\n{out_log}"
    );
}

fn reap(output: &mut Output) -> String {
    String::from_utf8_lossy(&output.stdout).to_string()
}

fn await_json(directory: &std::path::Path, generation: u64) -> (Output, serde_json::Value) {
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
            "20s",
        ],
    );
    let stdout = String::from_utf8_lossy(&output.stdout);
    let payload = stdout
        .split_once("Error:")
        .map_or(stdout.as_ref(), |(json, _)| json);
    let value = serde_json::from_str(payload.trim())
        .unwrap_or_else(|error| panic!("invalid await JSON ({error}): {stdout}"));
    (output, value)
}

fn write_service_script(directory: &std::path::Path) {
    std::fs::write(
        directory.join("api.sh"),
        "#!/bin/sh\nif [ -f api.pid ] && kill -0 \"$(cat api.pid)\" 2>/dev/null; then echo overlap > api.overlap; exit 99; fi\nprintf '%s\n' \"$$\" > \"api.pid.tmp.$$\" && mv \"api.pid.tmp.$$\" api.pid\ntouch api.started\nwhile [ ! -f service.fail ]; do sleep 0.02; done\nexit 7\n",
    )
    .unwrap();
    let mut permissions = std::fs::metadata(directory.join("api.sh"))
        .unwrap()
        .permissions();
    permissions.set_mode(0o755);
    std::fs::set_permissions(directory.join("api.sh"), permissions).unwrap();
}

/// A service whose shell ignores TERM/INT: only SIGKILL escalation can reap
/// its process group. Used to prove forced-termination breadth.
fn write_stubborn_service_script(directory: &std::path::Path) {
    std::fs::write(
        directory.join("stubborn.sh"),
        "#!/bin/sh\nprintf '%s\\n' \"$$\" > \"stubborn.pid.tmp.$$\" && mv \"stubborn.pid.tmp.$$\" stubborn.pid\ntouch stubborn.started\ntrap '' TERM\ntrap '' INT\nwhile :; do sleep 0.02; done\n",
    )
    .unwrap();
    let mut permissions = std::fs::metadata(directory.join("stubborn.sh"))
        .unwrap()
        .permissions();
    permissions.set_mode(0o755);
    std::fs::set_permissions(directory.join("stubborn.sh"), permissions).unwrap();
}

/// Reads a service PID marker. The marker is written by the service
/// process, which may not have started yet (or may still be mid-write for
/// non-atomic shells), so this waits — bounded — until the file exists and
/// parses. Never unwrap a possibly-absent marker directly; call sites that
/// need the file before any service started should gate on the service's
/// own `.started` marker instead.
fn service_pid(directory: &std::path::Path, marker: &str) -> u32 {
    let deadline = std::time::Instant::now() + Duration::from_secs(60);
    loop {
        let read = std::fs::read_to_string(directory.join(marker));
        match read {
            Ok(content) => {
                if let Ok(pid) = content.trim().parse() {
                    return pid;
                }
                // Torn/empty write: keep waiting until the writer commits.
            }
            Err(err) if err.kind() == std::io::ErrorKind::NotFound => {
                // Service not started yet: keep waiting.
            }
            Err(err) => panic!("reading {marker} in {}: {err}", directory.display()),
        }
        assert!(
            std::time::Instant::now() < deadline,
            "timed out waiting for a parseable {marker} in {}",
            directory.display()
        );
        std::thread::sleep(Duration::from_millis(20));
    }
}

fn wait_until<F: FnMut() -> bool>(mut condition: F, description: &str) {
    // 60s is a load upper bound (not a sleep): parallel integration runs on
    // a busy machine can exceed 20s without indicating a real failure.
    let deadline = std::time::Instant::now() + Duration::from_secs(60);
    loop {
        if condition() {
            return;
        }
        assert!(
            std::time::Instant::now() < deadline,
            "timed out waiting for {description}"
        );
        std::thread::sleep(Duration::from_millis(20));
    }
}

fn process_alive(pid: u32) -> bool {
    Command::new("kill")
        .args(["-0", &pid.to_string()])
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .status()
        .map(|status| status.success())
        .unwrap_or(false)
}

/// Group-level liveness (TASK-0162): reaping proof must show the whole
/// process group vanished (`kill -0 -- -pgid`), not only the leader pid —
/// a surviving descendant keeping the pgid alive must fail the assertion.
fn group_alive(pgid: u32) -> bool {
    Command::new("kill")
        .args(["-0", &format!("-{pgid}")])
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .status()
        .map(|status| status.success())
        .unwrap_or(false)
}

/// Returns a non-zombie member of a process group on Linux. PID 1 can retain
/// zombie descendants after their direct parent was waited, and `kill(-pgid,
/// 0)` deliberately reports such a zombie-only group as present. Reporting a
/// non-zombie member keeps the assertion focused on leaked live work, not
/// PID 1's eventual zombie reaping policy. Inspection failure is conservative and
/// counts as a live member, never as proof of cleanup.
#[cfg(target_os = "linux")]
fn group_has_non_zombie_member(pgid: u32) -> bool {
    let Ok(entries) = std::fs::read_dir("/proc") else {
        return true;
    };
    for entry in entries.flatten() {
        let Some(pid) = entry.file_name().to_string_lossy().parse::<u32>().ok() else {
            continue;
        };
        let Ok(stat) = std::fs::read_to_string(entry.path().join("stat")) else {
            continue;
        };
        let Some((_, fields)) = stat.rsplit_once(") ") else {
            continue;
        };
        let mut fields = fields.split_whitespace();
        let Some(state) = fields.next() else {
            continue;
        };
        let Some(_parent) = fields.next() else {
            continue;
        };
        let Some(process_group) = fields.next().and_then(|value| value.parse::<u32>().ok()) else {
            continue;
        };
        if process_group == pgid && state != "Z" {
            return true;
        }
    }
    false
}

/// Parses `scheduled generation: N` from a control CLI output; the panic
/// includes the full output so a flaky failure shows the real server error.
fn scheduled_generation(output: &Output) -> u64 {
    let stdout = String::from_utf8_lossy(&output.stdout);
    let combined = format!("{}{}", stdout, String::from_utf8_lossy(&output.stderr));
    combined
        .lines()
        .find_map(|line| line.strip_prefix("scheduled generation: "))
        .expect(&format!("scheduled generation line missing in: {combined}"))
        .parse()
        .expect("numeric generation")
}

#[test]
fn readiness_service_settles_generation_and_remains_in_pool() {
    let directory = setup_directory(
        "readiness-pass",
        r#"
on:
  socket: sock
hooks:
  success: 'echo "$FUNZZY_GENERATION_ID:$FUNZZY_GENERATION_OUTCOME" >> hook.log'
  failure: 'echo "failure:$FUNZZY_GENERATION_ID:$FUNZZY_GENERATION_OUTCOME" >> hook.log'
jobs:
  - name: api
    service: true
    run: ./api.sh
    change: "src/**"
    readiness:
      run: "test -f readiness.ok"
      timeout: 5s
      interval: 20ms
"#,
    );
    write_service_script(&directory);
    let _watcher = start_watcher(&directory);
    wait_until_socket(&directory);
    let socket = directory.join("sock");

    let scheduled = run_cli(&directory, &["control", "run", "api"]);
    assert!(
        scheduled.status.success(),
        "schedule: {}",
        reap(&mut scheduled.clone())
    );
    let generation = scheduled_generation(&scheduled);
    wait_until(
        || directory.join("api.started").exists(),
        "readiness service to start",
    );
    std::fs::write(directory.join("readiness.ok"), "ready").unwrap();

    wait_until_status(&socket, |status| {
        status["generation"].as_u64() == Some(generation)
            && status["state"].as_str() == Some("passed")
            && status["services"].as_array().is_some_and(|services| {
                services
                    .iter()
                    .any(|service| service["name"] == "api" && service["state"] == "ready")
            })
    });
    let status_snapshot = try_status(&socket).unwrap()["result"].clone();
    assert_eq!(status_snapshot["state"], "passed");
    assert_eq!(status_snapshot["services"][0]["name"], "api");
    assert_eq!(status_snapshot["services"][0]["state"], "ready");

    let (awaited, observation) = await_json(&directory, generation);
    assert!(
        awaited.status.success(),
        "await: {}",
        reap(&mut awaited.clone())
    );
    assert_eq!(observation["terminalReason"], "passed");
    assert_eq!(observation["snapshot"]["state"], "passed");
    let log = std::fs::read_to_string(directory.join("child.err")).unwrap();
    let summary = "Success; Completed: 1; Failed: 0; Duration:";
    assert_eq!(
        log.matches(summary).count(),
        1,
        "service-only settlement must print one completed summary: {log}"
    );
    wait_until(
        || {
            std::fs::read_to_string(directory.join("hook.log"))
                .map(|contents| contents.lines().count() == 1)
                .unwrap_or(false)
        },
        "success hook to run once at readiness settlement",
    );
    assert_eq!(
        std::fs::read_to_string(directory.join("hook.log"))
            .unwrap()
            .trim(),
        format!("{generation}:passed")
    );

    let pid = std::fs::read_to_string(directory.join("api.pid"))
        .unwrap()
        .trim()
        .parse::<u32>()
        .unwrap();
    assert!(process_alive(pid), "ready service must remain alive");
}

#[test]
fn post_settlement_service_failure_keeps_generation_terminal() {
    let directory = setup_directory(
        "readiness-post-failure",
        r#"
on:
  socket: sock
hooks:
  success: 'echo "$FUNZZY_GENERATION_ID:$FUNZZY_GENERATION_OUTCOME" >> hook.log'
  failure: 'echo "failure:$FUNZZY_GENERATION_ID:$FUNZZY_GENERATION_OUTCOME" >> hook.log'
jobs:
  - name: api
    service: true
    run: ./api.sh
    change: "src/**"
    readiness:
      run: "test -f readiness.ok"
      timeout: 5s
      interval: 20ms
  - name: quick
    run: "true"
    change: "src/**"
"#,
    );
    write_service_script(&directory);
    let _watcher = start_watcher(&directory);
    wait_until_socket(&directory);
    let socket = directory.join("sock");

    let scheduled = run_cli(&directory, &["control", "run", "api"]);
    assert!(
        scheduled.status.success(),
        "schedule: {}",
        reap(&mut scheduled.clone())
    );
    let service_generation = scheduled_generation(&scheduled);
    wait_until(
        || directory.join("api.started").exists(),
        "readiness service to start",
    );
    std::fs::write(directory.join("readiness.ok"), "ready").unwrap();
    wait_until_status(&socket, |status| {
        status["generation"].as_u64() == Some(service_generation)
            && status["state"].as_str() == Some("passed")
            && status["services"].as_array().is_some_and(|services| {
                services
                    .iter()
                    .any(|service| service["name"] == "api" && service["state"] == "ready")
            })
    });

    let unrelated = run_cli(&directory, &["control", "run", "quick"]);
    assert!(
        unrelated.status.success(),
        "schedule: {}",
        reap(&mut unrelated.clone())
    );
    let unrelated_generation = scheduled_generation(&unrelated);
    wait_until_status(&socket, |status| {
        status["generation"].as_u64() == Some(unrelated_generation)
            && status["state"].as_str() == Some("passed")
            && status["services"].as_array().is_some_and(|services| {
                services
                    .iter()
                    .any(|service| service["name"] == "api" && service["state"] == "ready")
            })
    });

    std::fs::write(directory.join("service.fail"), "fail").unwrap();
    wait_until_status(&socket, |status| {
        status["generation"].as_u64() == Some(unrelated_generation)
            && status["state"].as_str() == Some("passed")
            && status["services"].as_array().is_some_and(|services| {
                services
                    .iter()
                    .any(|service| service["name"] == "api" && service["state"] == "failed")
            })
    });
    let terminal = await_json(&directory, service_generation).1;
    assert_eq!(terminal["terminalReason"], "passed");
    assert_eq!(terminal["snapshot"]["state"], "passed");
    // TASK-0162 hook/output agreement: exactly two settlement summaries may
    // exist (the service-only generation and the unrelated quick generation,
    // each printed once at its boundary). A post-settlement service failure
    // must not produce a third summary or any contradictory hook history.
    let log = std::fs::read_to_string(directory.join("child.err")).unwrap();
    let summary = "Success; Completed: 1; Failed: 0; Duration:";
    assert_eq!(
        log.matches(summary).count(),
        2,
        "post-settlement service failure must not rerun settlement output: {log}"
    );
    wait_until(
        || {
            std::fs::read_to_string(directory.join("hook.log"))
                .map(|contents| contents.lines().count() == 2)
                .unwrap_or(false)
        },
        "success hooks for both settled generations",
    );
    let hooks = std::fs::read_to_string(directory.join("hook.log")).unwrap();
    assert_eq!(
        hooks.lines().count(),
        2,
        "post-settlement service failure must not invoke a failure hook: {hooks}"
    );
    assert!(
        hooks.lines().all(|line| line.ends_with(":passed")),
        "only the two passed generation hooks are expected: {hooks}"
    );
}

#[test]
fn cancelling_starting_readiness_service_reaps_service() {
    let directory = setup_directory(
        "readiness-cancel",
        r#"
on:
  socket: sock
jobs:
  - name: api
    service: true
    run: ./api.sh
    change: "src/**"
    readiness:
      run: "test -f readiness.ok"
      timeout: 30s
      interval: 20ms
"#,
    );
    write_service_script(&directory);
    let _watcher = start_watcher(&directory);
    wait_until_socket(&directory);
    let scheduled = run_cli(&directory, &["control", "run", "api"]);
    assert!(
        scheduled.status.success(),
        "schedule: {}",
        reap(&mut scheduled.clone())
    );
    let generation = scheduled_generation(&scheduled);
    wait_until(
        || directory.join("api.started").exists(),
        "cancelled readiness service to start",
    );
    let cancelled = run_cli(
        &directory,
        &[
            "control",
            "cancel",
            "--generation",
            &generation.to_string(),
            "--wait",
            "--timeout",
            "10s",
        ],
    );
    assert_eq!(
        cancelled.status.code(),
        Some(0),
        "cancel: {}",
        reap(&mut cancelled.clone())
    );
    let terminal = await_json(&directory, generation).1;
    assert_eq!(terminal["terminalReason"], "cancelled");
    let pid = std::fs::read_to_string(directory.join("api.pid"))
        .unwrap()
        .trim()
        .parse::<u32>()
        .unwrap();
    wait_until(|| !process_alive(pid), "cancelled service to be reaped");
}

#[test]
fn watcher_shutdown_reaps_ready_service() {
    let directory = setup_directory(
        "readiness-shutdown",
        r#"
on:
  socket: sock
jobs:
  - name: api
    service: true
    run: ./api.sh
    change: "src/**"
    readiness:
      run: "test -f readiness.ok"
      timeout: 5s
      interval: 20ms
"#,
    );
    write_service_script(&directory);
    let mut watcher = start_watcher(&directory);
    wait_until_socket(&directory);
    let scheduled = run_cli(&directory, &["control", "run", "api"]);
    assert!(
        scheduled.status.success(),
        "schedule: {}",
        reap(&mut scheduled.clone())
    );
    let generation = scheduled_generation(&scheduled);
    wait_until(
        || directory.join("api.started").exists(),
        "service to start",
    );
    std::fs::write(directory.join("readiness.ok"), "ready").unwrap();
    let socket = directory.join("sock");
    wait_until_status(&socket, |status| {
        status["generation"].as_u64() == Some(generation)
            && status["state"].as_str() == Some("passed")
    });
    let pid = std::fs::read_to_string(directory.join("api.pid"))
        .unwrap()
        .trim()
        .parse::<u32>()
        .unwrap();

    libc_kill(watcher.child.id() as i32, 15).expect("signal watcher shutdown");
    wait_until(
        || watcher.child.try_wait().ok().flatten().is_some(),
        "watcher to exit",
    );
    wait_until(|| !process_alive(pid), "shutdown service to be reaped");
}

#[test]
fn selecting_ready_service_replaces_without_overlap() {
    let directory = setup_directory(
        "readiness-replace",
        r#"
on:
  socket: sock
jobs:
  - name: api
    service: true
    run: ./api.sh
    change: "src/**"
    readiness:
      run: "test -f readiness.ok"
      timeout: 5s
      interval: 20ms
"#,
    );
    write_service_script(&directory);
    let _watcher = start_watcher(&directory);
    wait_until_socket(&directory);
    let socket = directory.join("sock");
    std::fs::write(directory.join("readiness.ok"), "ready").unwrap();

    let first = run_cli(&directory, &["control", "run", "api"]);
    assert!(
        first.status.success(),
        "schedule: {}",
        reap(&mut first.clone())
    );
    let first_generation = scheduled_generation(&first);
    wait_until_status(&socket, |status| {
        status["generation"].as_u64() == Some(first_generation)
            && status["state"].as_str() == Some("passed")
            && status["services"].as_array().is_some_and(|services| {
                services
                    .iter()
                    .any(|service| service["name"] == "api" && service["state"] == "ready")
            })
    });
    wait_until(
        || directory.join("api.pid").exists(),
        "first service PID marker",
    );
    let first_pid = std::fs::read_to_string(directory.join("api.pid"))
        .unwrap()
        .trim()
        .parse::<u32>()
        .unwrap();

    let second = run_cli(&directory, &["control", "run", "api"]);
    assert!(
        second.status.success(),
        "schedule: {}",
        reap(&mut second.clone())
    );
    let second_generation = scheduled_generation(&second);
    wait_until_status(&socket, |status| {
        status["generation"].as_u64() == Some(second_generation)
            && status["state"].as_str() == Some("passed")
            && status["services"].as_array().is_some_and(|services| {
                services
                    .iter()
                    .any(|service| service["name"] == "api" && service["state"] == "ready")
            })
    });
    let second_pid = std::fs::read_to_string(directory.join("api.pid"))
        .unwrap()
        .trim()
        .parse::<u32>()
        .unwrap();
    assert_ne!(
        first_pid, second_pid,
        "replacement must allocate a new process"
    );
    assert!(
        !directory.join("api.overlap").exists(),
        "same-name services overlapped"
    );
    assert!(!process_alive(first_pid), "old service must be reaped");
    assert!(process_alive(second_pid), "replacement must remain alive");
}

#[test]
fn readiness_timeout_fails_generation_and_reaps_service() {
    let directory = setup_directory(
        "readiness-fail",
        r#"
on:
  socket: sock
jobs:
  - name: api
    service: true
    run: ./api.sh
    change: "src/**"
    readiness:
      run: "false"
      timeout: 5s
      interval: 20ms
"#,
    );
    write_service_script(&directory);
    let _watcher = start_watcher(&directory);
    wait_until_socket(&directory);

    let scheduled = run_cli(&directory, &["control", "run", "api"]);
    assert!(
        scheduled.status.success(),
        "schedule: {}",
        reap(&mut scheduled.clone())
    );
    let generation = scheduled_generation(&scheduled);
    wait_until(
        || directory.join("api.started").exists(),
        "readiness service to start",
    );
    let (awaited, observation) = await_json(&directory, generation);
    assert_eq!(awaited.status.code(), Some(1), "failed await must exit 1");
    assert_eq!(observation["terminalReason"], "failed");
    assert_eq!(observation["snapshot"]["state"], "failed");

    let pid = std::fs::read_to_string(directory.join("api.pid"))
        .unwrap()
        .trim()
        .parse::<u32>()
        .unwrap();
    wait_until(
        || !process_alive(pid),
        "failed readiness service to be reaped",
    );
}

/// TASK-0162 AC: a file change that supersedes the settled generation must
/// replace the ready pooled service — old group reaped, replacement alive,
/// and no same-name overlap. This covers supersession triggered by real
/// filesystem events rather than explicit `control run` selection.
#[test]
fn file_change_supersession_replaces_ready_service_without_overlap() {
    let directory = setup_directory(
        "readiness-file-supersede",
        r#"
on:
  socket: sock
jobs:
  - name: api
    service: true
    run: ./api.sh
    change: "src/**"
    readiness:
      run: "test -f readiness.ok"
      timeout: 5s
      interval: 20ms
"#,
    );
    write_service_script(&directory);
    std::fs::create_dir_all(directory.join("src")).unwrap();
    // Instant promotion: readiness passes on the first probe.
    std::fs::write(directory.join("readiness.ok"), "ready").unwrap();
    let _watcher = start_watcher(&directory);
    wait_until_socket(&directory);
    let socket = directory.join("sock");

    std::fs::write(directory.join("src/first.rs"), "one").unwrap();
    wait_until_status(&socket, |status| {
        status["state"].as_str() == Some("passed")
            && status["services"].as_array().is_some_and(|services| {
                services
                    .iter()
                    .any(|service| service["name"] == "api" && service["state"] == "ready")
            })
    });
    let first_pid = service_pid(&directory, "api.pid");
    assert!(process_alive(first_pid), "first service must be alive");

    // Supersede with a new filesystem event selecting the same service.
    std::fs::write(directory.join("src/second.rs"), "two").unwrap();
    wait_until(
        || service_pid(&directory, "api.pid") != first_pid,
        "superseding generation to replace the service process",
    );
    let second_pid = service_pid(&directory, "api.pid");
    assert!(
        !process_alive(first_pid),
        "superseded service must be reaped"
    );
    assert!(process_alive(second_pid), "replacement must remain alive");
    assert!(
        !directory.join("api.overlap").exists(),
        "same-name services overlapped during supersession"
    );
}

/// TASK-0162 AC: SIGINT must reap the ready pooled service exactly like the
/// proven SIGTERM path — both signals share one shutdown coordinator.
#[test]
fn sigint_shutdown_reaps_ready_service() {
    let directory = setup_directory(
        "readiness-sigint",
        r#"
on:
  socket: sock
jobs:
  - name: api
    service: true
    run: ./api.sh
    change: "src/**"
    readiness:
      run: "test -f readiness.ok"
      timeout: 5s
      interval: 20ms
"#,
    );
    write_service_script(&directory);
    let mut watcher = start_watcher(&directory);
    wait_until_socket(&directory);
    let scheduled = run_cli(&directory, &["control", "run", "api"]);
    assert!(
        scheduled.status.success(),
        "schedule: {}",
        reap(&mut scheduled.clone())
    );
    let generation = scheduled_generation(&scheduled);
    wait_until(
        || directory.join("api.started").exists(),
        "service to start",
    );
    std::fs::write(directory.join("readiness.ok"), "ready").unwrap();
    let socket = directory.join("sock");
    wait_until_status(&socket, |status| {
        status["generation"].as_u64() == Some(generation)
            && status["state"].as_str() == Some("passed")
    });
    let pid = service_pid(&directory, "api.pid");

    libc_kill(watcher.child.id() as i32, 2).expect("signal watcher SIGINT shutdown");
    wait_until(
        || watcher.child.try_wait().ok().flatten().is_some(),
        "watcher to exit after SIGINT",
    );
    wait_until(
        || !process_alive(pid),
        "SIGINT shutdown service to be reaped",
    );
}

/// TASK-0162 AC: a service process group that ignores the graceful signal
/// must be force-killed (SIGKILL escalation) and reaped at watcher shutdown.
#[test]
fn term_ignoring_service_shutdown_escalates_and_reaps() {
    let directory = setup_directory(
        "readiness-escalation",
        r#"
on:
  socket: sock
jobs:
  - name: stubborn
    service: true
    run: ./stubborn.sh
    change: "src/**"
    readiness:
      run: "test -f readiness.ok"
      timeout: 5s
      interval: 20ms
"#,
    );
    write_stubborn_service_script(&directory);
    // Keep the graceful grace short so escalation is exercised quickly and
    // deterministically instead of waiting the default five seconds.
    let mut watcher = start_watcher_with_env(&directory, &[("FUNZZY_CANCEL_GRACE_MS", "300")]);
    wait_until_socket(&directory);
    let scheduled = run_cli(&directory, &["control", "run", "stubborn"]);
    assert!(
        scheduled.status.success(),
        "schedule: {}",
        reap(&mut scheduled.clone())
    );
    let generation = scheduled_generation(&scheduled);
    wait_until(
        || directory.join("stubborn.started").exists(),
        "stubborn service to start",
    );
    std::fs::write(directory.join("readiness.ok"), "ready").unwrap();
    let socket = directory.join("sock");
    wait_until_status(&socket, |status| {
        status["generation"].as_u64() == Some(generation)
            && status["state"].as_str() == Some("passed")
            && status["services"].as_array().is_some_and(|services| {
                services
                    .iter()
                    .any(|service| service["name"] == "stubborn" && service["state"] == "ready")
            })
    });
    let pid = service_pid(&directory, "stubborn.pid");
    assert!(
        process_alive(pid),
        "stubborn service must be alive pre-shutdown"
    );
    assert!(
        group_alive(pid),
        "stubborn service group must be alive pre-shutdown"
    );

    libc_kill(watcher.child.id() as i32, 15).expect("signal watcher shutdown");
    wait_until(
        || watcher.child.try_wait().ok().flatten().is_some(),
        "watcher to exit",
    );
    // The watcher must exit through the graceful signal path (128+15),
    // which waits for owned children — never a crash skip of the reap.
    assert_eq!(
        watcher.child.wait().expect("watcher reaped").code(),
        Some(143),
        "watcher exit must be the graceful SIGTERM path"
    );
    wait_until(
        || !process_alive(pid),
        "TERM-ignoring service leader to be force-killed and reaped",
    );
    // Linux PID 1 may retain zombie-only process groups after the direct
    // child was waited. `kill(-pgid, 0)` still reports those zombies, so the
    // portable proof is no non-zombie member; macOS has no equivalent stable
    // /proc state source and retains strict group disappearance.
    #[cfg(target_os = "linux")]
    wait_until(
        || !group_has_non_zombie_member(pid),
        "TERM-ignoring service group to have no non-zombie member",
    );
    #[cfg(not(target_os = "linux"))]
    wait_until(
        || !group_alive(pid),
        "TERM-ignoring service process group to fully disappear",
    );
}

/// TASK-0162 AC: exact cancellation of a generation that is replacing a
/// ready pooled service. The replacement reaps the old group first; the
/// cancellation then stops whatever replacement state exists without
/// overlap or leaked processes.
#[test]
fn cancelling_replacement_generation_reaps_without_overlap() {
    let directory = setup_directory(
        "readiness-cancel-replacement",
        r#"
on:
  socket: sock
jobs:
  - name: api
    service: true
    run: ./api.sh
    change: "src/**"
    readiness:
      run: "test -f readiness.ok"
      timeout: 30s
      interval: 20ms
"#,
    );
    write_service_script(&directory);
    std::fs::write(directory.join("readiness.ok"), "ready").unwrap();
    let _watcher = start_watcher(&directory);
    wait_until_socket(&directory);
    let socket = directory.join("sock");

    let first = run_cli(&directory, &["control", "run", "api"]);
    assert!(
        first.status.success(),
        "schedule: {}",
        reap(&mut first.clone())
    );
    let first_generation = scheduled_generation(&first);
    wait_until_status(&socket, |status| {
        status["generation"].as_u64() == Some(first_generation)
            && status["state"].as_str() == Some("passed")
            && status["services"].as_array().is_some_and(|services| {
                services
                    .iter()
                    .any(|service| service["name"] == "api" && service["state"] == "ready")
            })
    });
    let first_pid = service_pid(&directory, "api.pid");
    assert!(process_alive(first_pid), "first service must be alive");

    // Remove the readiness marker so the replacement generation cannot
    // settle before the cancellation lands.
    std::fs::remove_file(directory.join("readiness.ok")).unwrap();
    let second = run_cli(&directory, &["control", "run", "api"]);
    assert!(
        second.status.success(),
        "schedule: {}",
        reap(&mut second.clone())
    );
    let second_generation = scheduled_generation(&second);
    // Deterministic barrier: the replacement only starts after the old group
    // is physically reaped (contract: stop/reap-before-start).
    wait_until(
        || !process_alive(first_pid),
        "replacement to reap the superseded service group",
    );

    let cancelled = run_cli(
        &directory,
        &[
            "control",
            "cancel",
            "--generation",
            &second_generation.to_string(),
            "--wait",
            "--timeout",
            "10s",
        ],
    );
    assert_eq!(
        cancelled.status.code(),
        Some(0),
        "cancel: {}",
        reap(&mut cancelled.clone())
    );
    let terminal = await_json(&directory, second_generation).1;
    assert_eq!(terminal["terminalReason"], "cancelled");
    assert_eq!(terminal["snapshot"]["state"], "cancelled");

    // Whatever replacement state existed, it must be reaped by the cancel.
    if service_pid(&directory, "api.pid") != first_pid {
        let replacement_pid = service_pid(&directory, "api.pid");
        wait_until(
            || !process_alive(replacement_pid),
            "cancelled replacement service to be reaped",
        );
    }
    assert!(
        !directory.join("api.overlap").exists(),
        "same-name services overlapped during cancelled replacement"
    );
}

const INIT_FAST: &str = r#"
on:
  socket: sock
tasks:
  - name: init task
    run: "true"
    change: "*.txt"
    run_on_init: true
"#;

const NO_INIT: &str = r#"
on:
  socket: sock
tasks:
  - name: long running
    run: "sleep 6"
    change: "*.txt"
  - name: quick
    run: "true"
    change: "*.txt"
"#;

/// Fast-only matching config: any `.txt` change runs an instant task, so a
/// scheduled generation reaches terminal immediately.
const FAST_ONLY: &str = r#"
on:
  socket: sock
tasks:
  - name: quick
    run: "true"
    change: "*.txt"
"#;

const DURATION_PARITY: &str = r#"
on:
  socket: sock
jobs:
  - name: serial first
    run: "true"
    change: "*.txt"
    run_on_init: true
  - name: serial second
    run: "true"
    change: "*.txt"
    run_on_init: true
"#;

fn start_watcher_with_events(directory: &std::path::Path, events: &std::path::Path) -> TestProcess {
    let child_log = std::fs::File::create(directory.join("child.err")).unwrap();
    let child = Command::new(env!("CARGO_BIN_EXE_fzz"))
        .current_dir(directory)
        .env_remove("FUNZZY_BAIL")
        .env_remove("FUNZZY_NON_BLOCK")
        .args(["--events", events.to_str().unwrap()])
        .stdout(Stdio::from(child_log.try_clone().unwrap()))
        .stderr(Stdio::from(child_log))
        .spawn()
        .unwrap();
    TestProcess {
        child,
        directory: directory.to_path_buf(),
    }
}

#[test]
fn control_snapshot_human_rows_and_terminal_events_agree() {
    let directory = setup_directory("duration-parity", DURATION_PARITY);
    let events = directory.join("events.ndjson");
    let _watcher = start_watcher_with_events(&directory, &events);
    wait_until_socket(&directory);
    let socket = directory.join("sock");
    wait_until_status(&socket, |status| {
        status["generation"].as_u64() == Some(1)
            && status["state"].as_str() == Some("passed")
            && status["tasks"]
                .as_array()
                .is_some_and(|tasks| tasks.len() == 2)
    });

    let snapshot = try_status(&socket).unwrap()["result"].clone();
    let tasks = snapshot["tasks"].as_array().unwrap();
    assert_eq!(tasks[0]["name"], "serial first");
    assert_eq!(tasks[1]["name"], "serial second");
    assert!(tasks.iter().all(|task| task["durationMs"].is_u64()));

    let human = run_cli(&directory, &["control", "status"]);
    let human = String::from_utf8_lossy(&human.stdout);
    assert!(
        human.contains("jobs:") && human.contains("DURATION"),
        "{human}"
    );
    assert!(
        human.find("serial first") < human.find("serial second"),
        "human declaration order: {human}"
    );

    let records: Vec<serde_json::Value> = std::fs::read_to_string(&events)
        .unwrap()
        .lines()
        .map(|line| serde_json::from_str::<serde_json::Value>(line).unwrap())
        .filter(|record| record["event"] == "task_terminal")
        .collect();
    assert_eq!(records.len(), 2);
    for (task, event) in tasks.iter().zip(records) {
        assert_eq!(event["task"], task["name"]);
        assert_eq!(event["state"], task["state"]);
        assert_eq!(event["durationMs"], task["durationMs"]);
    }
}

#[test]
fn await_already_terminal_generation_returns_immediately() {
    let directory = setup_directory("already-terminal", INIT_FAST);
    let _watcher = start_watcher(&directory);
    wait_until_socket(&directory);
    let socket = directory.join("sock");
    wait_until_status(&socket, |status| {
        status["generation"].as_u64() == Some(1) && status["state"].as_str() == Some("passed")
    });

    let output = run_cli(
        &directory,
        &["control", "await", "--generation", "1", "--timeout", "5s"],
    );
    assert!(
        output.status.success(),
        "already-terminal await exits 0: {}",
        reap(&mut output.clone())
    );
    let stdout = reap(&mut output.clone());
    assert!(
        stdout.contains("terminal reason: passed"),
        "stdout: {stdout}"
    );
    assert!(stdout.contains("freshness: current"), "stdout: {stdout}");
    assert!(stdout.contains("generation: 1"), "stdout: {stdout}");
}

#[test]
fn await_future_completion_blocks_then_returns() {
    let directory = setup_directory("future", INIT_FAST);
    let _watcher = start_watcher(&directory);
    wait_until_socket(&directory);
    let socket = directory.join("sock");
    wait_until_status(&socket, |status| status["generation"].as_u64() == Some(1));

    // A second generation is triggered by a file write; awaiting it must
    // block until it reaches terminal, then return passed with exit 0.
    std::fs::write(directory.join("notes.txt"), "x").unwrap();
    wait_until_status(&socket, |status| status["generation"].as_u64() == Some(2));

    let output = run_cli(
        &directory,
        &["control", "await", "--generation", "2", "--timeout", "10s"],
    );
    assert!(output.status.success());
    let stdout = reap(&mut output.clone());
    assert!(
        stdout.contains("terminal reason: passed"),
        "stdout: {stdout}"
    );
    assert!(stdout.contains("generation: 2"), "stdout: {stdout}");
}

#[test]
fn await_no_generation_yet_times_out_with_latest_snapshot() {
    let directory = setup_directory("no-gen", NO_INIT);
    let _watcher = start_watcher(&directory);
    wait_until_socket(&directory);

    let output = run_cli(
        &directory,
        &["control", "await", "--after", "0", "--timeout", "800ms"],
    );
    assert_eq!(output.status.code(), Some(1));
    let stdout = reap(&mut output.clone());
    assert!(
        stdout.contains("terminal reason: timeout"),
        "stdout: {stdout}"
    );
    assert!(stdout.contains("generation: 0"), "stdout: {stdout}");
    assert!(stdout.contains("state: idle"), "stdout: {stdout}");
}

#[test]
fn await_superseded_generation_returns_superseded() {
    let directory = setup_directory("superseded", NO_INIT);
    let _watcher = start_watcher(&directory);
    wait_until_socket(&directory);
    let socket = directory.join("sock");

    // Generation 1 runs for 30s; awaiting it while a new batch arrives must
    // return superseded with the newer snapshot, exit 1.
    let first = run_cli_retry(&directory, &["control", "emit", "a.txt"]);
    if !first.status.success() {
        let watcher_log = std::fs::read_to_string(directory.join("child.err"))
            .unwrap_or_else(|_| "(no watcher stderr)".to_string());
        panic!(
            "first emit failed: {}; watcher stderr:\n{}",
            String::from_utf8_lossy(&first.stdout),
            watcher_log
        );
    }
    let run_one = scheduled_generation(&first);
    wait_until_status(&socket, |status| {
        status["generation"].as_u64() == Some(run_one)
            && status["state"].as_str() == Some("running")
    });

    let waiter = spawn_cli(
        &directory,
        &[
            "control",
            "await",
            "--generation",
            &run_one.to_string(),
            "--timeout",
            "20s",
        ],
    );
    std::thread::sleep(Duration::from_millis(1200));
    let second = run_cli_retry(&directory, &["control", "emit", "b.txt"]);
    let run_two = scheduled_generation(&second);
    assert!(run_two > run_one);

    let output = waiter.wait_with_output().expect("waiter finished");
    assert_eq!(output.status.code(), Some(1), "superseded await exits 1");
    let stdout = String::from_utf8_lossy(&output.stdout);
    assert!(
        stdout.contains("terminal reason: superseded"),
        "stdout: {stdout}"
    );
    assert!(
        stdout.contains(&format!("generation: {run_two}")),
        "stdout: {stdout}"
    );
}

#[test]
fn await_watcher_disconnect_reports_disconnected() {
    let directory = setup_directory("disconnect", NO_INIT);
    let mut watcher = start_watcher(&directory);
    wait_until_socket(&directory);

    let waiter = spawn_cli(
        &directory,
        &["control", "await", "--after", "0", "--timeout", "60s"],
    );
    std::thread::sleep(Duration::from_millis(1200));
    watcher.kill();

    let output = waiter.wait_with_output().expect("waiter finished");
    assert_eq!(output.status.code(), Some(1), "disconnected await exits 1");
    let combined = format!(
        "{}{}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    );
    assert!(combined.contains("disconnected"), "output: {combined}");
}

#[test]
fn await_watcher_restart_reports_restarted() {
    let directory = setup_directory("restart", INIT_FAST);
    let mut watcher = start_watcher(&directory);
    wait_until_socket(&directory);
    let socket = directory.join("sock");
    wait_until_status(&socket, |status| status["generation"].as_u64() == Some(1));

    let waiter = spawn_cli(
        &directory,
        &["control", "await", "--after", "1", "--timeout", "60s"],
    );
    std::thread::sleep(Duration::from_millis(1200));

    // Replace the instance at the same socket path before the client's
    // re-negotiation window closes.
    watcher.kill();
    let _ = std::fs::remove_file(&socket);
    let replacement = start_watcher(&directory);
    wait_until_socket(&directory);

    let output = waiter.wait_with_output().expect("waiter finished");
    assert_eq!(output.status.code(), Some(1), "restarted await exits 1");
    let combined = format!(
        "{}{}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    );
    assert!(combined.contains("restarted"), "output: {combined}");
    let _ = replacement;
}

#[test]
fn multiple_waiters_all_return_on_one_terminal_event() {
    let directory = setup_directory("multi-waiter", FAST_ONLY);
    let _watcher = start_watcher(&directory);
    wait_until_socket(&directory);

    let first = spawn_cli(
        &directory,
        &["control", "await", "--after", "0", "--timeout", "20s"],
    );
    let second = spawn_cli(
        &directory,
        &["control", "await", "--after", "0", "--timeout", "20s"],
    );
    std::thread::sleep(Duration::from_millis(800));

    let trigger = run_cli(&directory, &["control", "emit", "x.txt"]);
    assert!(trigger.status.success());

    for waiter in [first, second] {
        let output = waiter.wait_with_output().expect("waiter finished");
        assert!(output.status.success(), "waiter must exit 0");
        let stdout = String::from_utf8_lossy(&output.stdout);
        assert!(
            stdout.contains("terminal reason: passed"),
            "stdout: {stdout}"
        );
    }
}

#[test]
fn await_timeout_performs_no_cancellation() {
    let directory = setup_directory("timeout", NO_INIT);
    let _watcher = start_watcher(&directory);
    wait_until_socket(&directory);
    let socket = directory.join("sock");

    // A generation runs long; a short await on a future generation times out
    // and must NOT cancel the running work.
    let emit = run_cli_retry(&directory, &["control", "emit", "a.txt"]);
    let run_id = scheduled_generation(&emit);
    wait_until_status(&socket, |status| {
        status["generation"].as_u64() == Some(run_id) && status["state"].as_str() == Some("running")
    });

    let output = run_cli(
        &directory,
        &[
            "control",
            "await",
            "--generation",
            "999",
            "--timeout",
            "500ms",
        ],
    );
    assert_eq!(output.status.code(), Some(1));
    let stdout = reap(&mut output.clone());
    assert!(
        stdout.contains("terminal reason: timeout"),
        "stdout: {stdout}"
    );

    // The running generation is untouched.
    wait_until_status(&socket, |status| {
        status["generation"].as_u64() == Some(run_id) && status["state"].as_str() == Some("running")
    });
}

#[test]
fn control_run_wait_returns_one_observation() {
    let directory = setup_directory("run-wait", INIT_FAST);
    let _watcher = start_watcher(&directory);
    wait_until_socket(&directory);
    let socket = directory.join("sock");
    wait_until_status(&socket, |status| status["generation"].as_u64() == Some(1));

    let output = run_cli(
        &directory,
        &["control", "run", "init task", "--wait", "--timeout", "10s"],
    );
    assert!(output.status.success(), "run --wait passed exits 0");
    let stdout = reap(&mut output.clone());
    assert!(stdout.contains("scheduled generation:"), "stdout: {stdout}");
    assert!(
        stdout.contains("terminal reason: passed"),
        "stdout: {stdout}"
    );
    assert!(
        stdout.contains("jobs:") && stdout.contains("init task") && stdout.contains("DURATION"),
        "human control result includes the terminal job row: {stdout}"
    );
}

#[test]
fn control_run_wait_reports_failed_workflow() {
    let directory = setup_directory(
        "run-wait-failed",
        r#"
on:
  socket: sock
tasks:
  - name: failing target
    run: "false"
    change: "*.txt"
"#,
    );
    let _watcher = start_watcher(&directory);
    wait_until_socket(&directory);

    let output = run_cli(
        &directory,
        &[
            "control",
            "run",
            "failing target",
            "--wait",
            "--timeout",
            "10s",
        ],
    );
    assert_eq!(output.status.code(), Some(1), "failed workflow exits 1");
    let stdout = reap(&mut output.clone());
    assert!(
        stdout.contains("terminal reason: failed"),
        "stdout: {stdout}"
    );
}

#[test]
fn control_emit_wait_returns_one_observation() {
    let directory = setup_directory("emit-wait", FAST_ONLY);
    let _watcher = start_watcher(&directory);
    wait_until_socket(&directory);

    let output = run_cli(
        &directory,
        &["control", "emit", "notes.txt", "--wait", "--timeout", "10s"],
    );
    assert!(output.status.success(), "emit --wait passed exits 0");
    let stdout = reap(&mut output.clone());
    assert!(stdout.contains("outcome: scheduled"), "stdout: {stdout}");
    assert!(
        stdout.contains("terminal reason: passed"),
        "stdout: {stdout}"
    );
}

#[test]
fn control_emit_wait_noop_stays_explicit_noop() {
    let directory = setup_directory("emit-wait-noop", NO_INIT);
    let _watcher = start_watcher(&directory);
    wait_until_socket(&directory);

    let output = run_cli(
        &directory,
        &[
            "control",
            "emit",
            "src/main.rs",
            "--wait",
            "--timeout",
            "5s",
        ],
    );
    assert!(output.status.success(), "no-op emit --wait exits 0");
    let stdout = reap(&mut output.clone());
    assert!(stdout.contains("outcome: unmatched"), "stdout: {stdout}");
    assert!(
        !stdout.contains("terminal reason"),
        "no observation for a no-op: {stdout}"
    );
}

#[test]
fn control_await_usage_errors_exit_two() {
    let directory = setup_directory("usage", INIT_FAST);
    let _watcher = start_watcher(&directory);
    wait_until_socket(&directory);

    let missing_timeout = run_cli(&directory, &["control", "await", "--after", "1"]);
    assert_eq!(missing_timeout.status.code(), Some(2));

    let both_modes = run_cli(
        &directory,
        &[
            "control",
            "await",
            "--after",
            "1",
            "--generation",
            "2",
            "--timeout",
            "1s",
        ],
    );
    assert_eq!(both_modes.status.code(), Some(2));

    let bad_duration = run_cli(
        &directory,
        &["control", "await", "--after", "1", "--timeout", "1h"],
    );
    assert_eq!(bad_duration.status.code(), Some(2));
}
