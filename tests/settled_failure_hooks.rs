//! TASK-0154: black-box proof for settled failure hooks.
//!
//! These tests use only the installed `fzz` binary and the control socket. They
//! deliberately assert the invocation boundary rather than internal worker
//! state: settled hooks are shell commands with reserved generation context.

#![cfg(all(feature = "test-integration", unix))]

use serde_json::Value;
use std::io::{BufRead, BufReader, Write};
use std::os::unix::net::UnixStream;
use std::path::{Path, PathBuf};
use std::process::{Child, Command, Output, Stdio};
use std::sync::atomic::{AtomicU64, Ordering};
use std::time::{Duration, Instant};

static FIXTURE_ID: AtomicU64 = AtomicU64::new(0);

const CONFIG_PREFIX: &str = "on:\n  socket: sock\n";

fn fixture(label: &str, jobs: &str, hooks: &str) -> PathBuf {
    let sequence = FIXTURE_ID.fetch_add(1, Ordering::Relaxed);
    // Keep the socket path short: macOS limits Unix-domain paths to SUN_LEN.
    let root =
        std::env::temp_dir().join(format!("fzzsh-{}-{sequence}-{label}", std::process::id()));
    let _ = std::fs::remove_dir_all(&root);
    std::fs::create_dir_all(root.join("job-dir")).expect("create job cwd");
    std::fs::write(
        root.join(".watch.yaml"),
        format!("{CONFIG_PREFIX}{hooks}jobs:\n{jobs}"),
    )
    .expect("write config");
    std::fs::canonicalize(root).expect("canonicalize fixture")
}

struct Watcher {
    child: Child,
    root: PathBuf,
}

impl Watcher {
    fn start(root: &Path) -> Self {
        let stdout = std::fs::File::create(root.join("watcher.out")).expect("watcher stdout");
        let stderr = std::fs::File::create(root.join("watcher.err")).expect("watcher stderr");
        let child = Command::new(env!("CARGO_BIN_EXE_fzz"))
            .current_dir(root)
            .env("SETTLED_TEST_ENV", "inherited-value")
            .env("FUNZZY_GENERATION_ID", "inherited-id")
            .env("FUNZZY_GENERATION_OUTCOME", "inherited-outcome")
            .env_remove("FUNZZY_BAIL")
            .env_remove("FUNZZY_NON_BLOCK")
            .stdout(Stdio::from(stdout))
            .stderr(Stdio::from(stderr))
            .spawn()
            .expect("spawn watcher");
        Self {
            child,
            root: root.to_path_buf(),
        }
    }

    fn stop(&mut self) {
        if self.child.try_wait().expect("poll watcher").is_none() {
            let _ = unsafe { kill(self.child.id() as i32, 15) };
        }
        let _ = self.child.wait();
        let _ = std::fs::remove_dir_all(&self.root);
    }
}

impl Drop for Watcher {
    fn drop(&mut self) {
        self.stop();
    }
}

unsafe extern "C" {
    fn kill(pid: i32, signal: i32) -> i32;
}

fn wait_until(label: &str, timeout: Duration, mut condition: impl FnMut() -> bool) {
    let deadline = Instant::now() + timeout;
    while Instant::now() < deadline {
        if condition() {
            return;
        }
        std::thread::sleep(Duration::from_millis(25));
    }
    panic!("timed out waiting for {label}");
}

fn status(root: &Path) -> Option<Value> {
    let mut stream = UnixStream::connect(root.join("sock")).ok()?;
    writeln!(
        stream,
        r#"{{"jsonrpc":"2.0","id":"status","method":"status"}}"#
    )
    .ok()?;
    let mut line = String::new();
    BufReader::new(stream).read_line(&mut line).ok()?;
    serde_json::from_str(&line).ok()
}

fn wait_ready(root: &Path) {
    let socket = root.join("sock");
    let deadline = Instant::now() + Duration::from_secs(20);
    let mut last_error = String::new();
    while Instant::now() < deadline {
        match UnixStream::connect(&socket) {
            Ok(_) => return,
            Err(error) => last_error = error.to_string(),
        }
        std::thread::sleep(Duration::from_millis(25));
    }
    panic!(
        "timed out waiting for control socket at {} (exists={}, error={}): config={}; out={}; err={}",
        socket.display(),
        socket.exists(),
        last_error,
        std::fs::read_to_string(root.join(".watch.yaml")).unwrap_or_default(),
        lines(root, "watcher.out"),
        lines(root, "watcher.err")
    );
}

fn run_cli(root: &Path, args: &[&str]) -> Output {
    Command::new(env!("CARGO_BIN_EXE_fzz"))
        .current_dir(root)
        .args(args)
        .output()
        .expect("run fzz control client")
}

fn scheduled_generation(output: &Output) -> u64 {
    let text = format!(
        "{}{}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    );
    text.lines()
        .find_map(|line| line.strip_prefix("scheduled generation: "))
        .unwrap_or_else(|| panic!("missing scheduled generation in {text}"))
        .parse()
        .expect("numeric generation")
}

fn run_and_wait(root: &Path, target: &str, expected_state: &str) -> u64 {
    let run = run_cli(root, &["control", "run", target]);
    let generation = scheduled_generation(&run);
    wait_until(
        "exact generation terminal state",
        Duration::from_secs(10),
        || {
            status(root).is_some_and(|reply| {
                reply["result"]["generation"].as_u64() == Some(generation)
                    && reply["result"]["state"].as_str() == Some(expected_state)
            })
        },
    );
    generation
}

fn lines(root: &Path, name: &str) -> String {
    std::fs::read_to_string(root.join(name)).unwrap_or_default()
}

fn wait_for_text(root: &Path, file: &str, text: &str, timeout: Duration) {
    wait_until(text, timeout, || lines(root, file).contains(text));
}

#[test]
fn stable_failure_runs_settled_hook_once_and_exposes_invocation_boundary() {
    let root = fixture(
        "stable",
        "  - name: fail\n    run: \"echo task-failure >&2; exit 3\"\n    cwd: job-dir\n    change: \"*.txt\"\n",
        "hooks:\n  failure:\n    run: 'echo hook >> settled.log; echo $PWD >> settled.log; echo $SETTLED_TEST_ENV >> settled.log; echo $FUNZZY_GENERATION_ID >> settled.log; echo $FUNZZY_GENERATION_OUTCOME >> settled.log; echo hook-output'\n    settle: 1s\n",
    );
    let mut watcher = Watcher::start(&root);
    wait_ready(&root);

    let generation = run_and_wait(&root, "fail", "failed");
    let deadline = Instant::now() + Duration::from_secs(10);
    while Instant::now() < deadline && lines(&root, "settled.log").lines().count() < 5 {
        std::thread::sleep(Duration::from_millis(25));
    }
    assert_eq!(
        lines(&root, "settled.log").lines().count(),
        5,
        "settled hook did not finish exactly once; generation={generation}, status={:?}, out={}, err={}",
        status(&root),
        lines(&root, "watcher.out"),
        lines(&root, "watcher.err")
    );

    let invocation = lines(&root, "settled.log");
    assert_eq!(
        invocation.lines().count(),
        5,
        "generation {generation}: {invocation}"
    );
    let fields: Vec<_> = invocation.lines().collect();
    assert_eq!(fields.first().copied(), Some("hook"));
    assert_eq!(
        fields.get(1).map(|value| Path::new(value)),
        Some(root.as_path())
    );
    assert_eq!(fields.get(2).copied(), Some("inherited-value"));
    assert_eq!(
        fields
            .get(3)
            .and_then(|value| value.trim().parse::<u64>().ok()),
        Some(generation)
    );
    assert_eq!(fields.get(4).map(|value| value.trim()), Some("failed"));
    assert!(
        lines(&root, "watcher.out").contains("hook-output"),
        "settled hook stdout must be forwarded"
    );
    assert_eq!(status(&root).unwrap()["result"]["generation"], generation);
    let awaited = run_cli(
        &root,
        &[
            "control",
            "await",
            "--generation",
            &generation.to_string(),
            "--timeout",
            "10s",
        ],
    );
    assert_eq!(
        awaited.status.code(),
        Some(1),
        "await preserves failed result"
    );
    assert!(
        String::from_utf8_lossy(&awaited.stdout).contains("terminal reason: failed"),
        "await must report the exact terminal result: {}",
        String::from_utf8_lossy(&awaited.stdout)
    );
    watcher.stop();
}

#[test]
fn newer_generation_starts_before_old_settlement_and_suppresses_old_hook() {
    let root = fixture(
        "supersede",
        "  - name: fail\n    run: \"exit 3\"\n    change: \"*.txt\"\n  - name: pass\n    run: \"echo replacement\"\n    change: \"*.txt\"\n",
        "hooks:\n  failure:\n    run: 'printf stale >> stale.log'\n    settle: 1440m\n",
    );
    let mut watcher = Watcher::start(&root);
    wait_ready(&root);

    let first = run_and_wait(&root, "fail", "failed");
    let second = run_and_wait(&root, "pass", "passed");
    assert!(second > first, "replacement must have a newer identity");
    assert!(
        !root.join("stale.log").exists(),
        "superseded hook must not run"
    );
    assert_eq!(status(&root).unwrap()["result"]["generation"], second);
    watcher.stop();
}

#[test]
fn repeated_failures_coalesce_and_newer_pass_suppresses_every_stale_hook() {
    let root = fixture(
        "coalesce",
        "  - name: fail\n    run: \"exit 3\"\n    change: \"*.txt\"\n  - name: pass\n    run: \"true\"\n    change: \"*.txt\"\n",
        "hooks:\n  failure:\n    run: 'printf stale >> stale.log'\n    settle: 1440m\n",
    );
    let mut watcher = Watcher::start(&root);
    wait_ready(&root);

    let first = run_and_wait(&root, "fail", "failed");
    let second = run_and_wait(&root, "fail", "failed");
    assert!(second > first);
    let third = run_and_wait(&root, "pass", "passed");
    assert!(third > second);
    assert!(
        !root.join("stale.log").exists(),
        "pass suppresses stale failures"
    );
    watcher.stop();
}

#[test]
fn shutdown_cancels_pending_hook_and_hook_failure_does_not_change_result() {
    let root = fixture(
        "shutdown",
        "  - name: fail\n    run: \"exit 3\"\n    change: \"*.txt\"\n",
        "hooks:\n  failure:\n    run: 'echo hook-command-failed >&2; exit 7'\n    settle: 1s\n",
    );
    let mut watcher = Watcher::start(&root);
    wait_ready(&root);
    let generation = run_and_wait(&root, "fail", "failed");
    let _ = unsafe { kill(watcher.child.id() as i32, 15) };
    let exit = watcher.child.wait().expect("wait watcher");
    assert_eq!(exit.code(), Some(143));
    assert_eq!(status(&root), None, "shutdown must retire the socket");
    assert!(!lines(&root, "watcher.err").contains("hook-command-failed"));
    assert_eq!(
        generation, 1,
        "the first controlled failure is generation one"
    );
    watcher.stop();
}

#[test]
fn failed_hook_is_observable_without_changing_failed_generation() {
    let root = fixture(
        "hook-failure",
        "  - name: fail\n    run: \"exit 3\"\n    change: \"*.txt\"\n",
        "hooks:\n  failure:\n    run: 'echo hook-command-failed >&2; exit 7'\n    settle: 1s\n",
    );
    let mut watcher = Watcher::start(&root);
    wait_ready(&root);
    let generation = run_and_wait(&root, "fail", "failed");
    wait_until("hook failure diagnostic", Duration::from_secs(10), || {
        lines(&root, "watcher.out").contains("settled failure hook")
    });
    let output = lines(&root, "watcher.out");
    assert!(
        output.contains("failed with"),
        "hook outcome must be visible: {output}"
    );
    assert_eq!(status(&root).unwrap()["result"]["generation"], generation);
    assert_eq!(status(&root).unwrap()["result"]["state"], "failed");
    watcher.stop();
}

#[test]
fn immediate_failure_hook_receives_reserved_generation_context() {
    let root = fixture(
        "immediate",
        "  - name: fail\n    run: \"exit 3\"\n    change: \"*.txt\"\n",
        "hooks:\n  failure: 'echo \"$FUNZZY_GENERATION_ID-$FUNZZY_GENERATION_OUTCOME\" > immediate.log'\n",
    );
    let mut watcher = Watcher::start(&root);
    wait_ready(&root);
    let generation = run_and_wait(&root, "fail", "failed");
    wait_until("immediate hook completion", Duration::from_secs(5), || {
        root.join("immediate.log").exists()
    });
    assert_eq!(
        lines(&root, "immediate.log"),
        format!("{generation}-failed\n")
    );
    watcher.stop();
}

#[test]
fn valid_reload_keeps_pending_hook_bound_to_failed_generation_snapshot() {
    let root = fixture(
        "reload",
        "  - name: fail\n    run: \"exit 3\"\n    change: \"*.txt\"\n",
        "hooks:\n  failure:\n    run: 'printf old >> old-hook.log'\n    settle: 5s\n",
    );
    let mut watcher = Watcher::start(&root);
    wait_ready(&root);
    let generation = run_and_wait(&root, "fail", "failed");
    std::fs::write(
        root.join(".watch.yaml"),
        "on:\n  socket: sock\nhooks:\n  failure:\n    run: 'printf new >> new-hook.log'\n    settle: 5s\njobs:\n  - name: fail\n    run: \"exit 3\"\n    change: \"*.txt\"\n",
    )
    .expect("write valid reload");
    wait_for_text(
        &root,
        "watcher.out",
        "hot-reloading to revision 2",
        Duration::from_secs(10),
    );
    wait_for_text(&root, "old-hook.log", "old", Duration::from_secs(10));
    assert!(!root.join("new-hook.log").exists());
    assert_eq!(status(&root).unwrap()["result"]["generation"], generation);
    watcher.stop();
}

#[test]
fn malformed_reload_cancels_pending_settlement_and_keeps_old_hook_unrun() {
    let root = fixture(
        "reload-invalid",
        "  - name: fail\n    run: \"exit 3\"\n    change: \"*.txt\"\n",
        "hooks:\n  failure:\n    run: 'printf stale >> stale-hook.log'\n    settle: 5s\n",
    );
    let mut watcher = Watcher::start(&root);
    wait_ready(&root);
    run_and_wait(&root, "fail", "failed");
    std::fs::write(root.join(".watch.yaml"), "jobs: [unclosed").expect("write malformed reload");

    let deadline = Instant::now() + Duration::from_secs(15);
    while Instant::now() < deadline && watcher.child.try_wait().expect("poll watcher").is_none() {
        std::thread::sleep(Duration::from_millis(25));
    }
    let exit = watcher.child.wait().expect("wait watcher");
    assert_eq!(exit.code(), Some(1), "malformed reload must be fatal");
    assert!(!root.join("stale-hook.log").exists());
    watcher.stop();
}

#[test]
fn finite_run_uses_failure_hook_immediately_without_settlement() {
    let root = fixture(
        "finite",
        "  - name: fail\n    run: \"exit 3\"\n    change: \"*.txt\"\n",
        "hooks:\n  failure:\n    run: 'printf unexpected >> unexpected.log'\n    settle: 1s\n",
    );
    let result = run_cli(&root, &["run", "fail"]);
    assert!(!result.status.success());
    assert!(
        root.join("unexpected.log").exists(),
        "finite run uses the failure command immediately, without a settlement timer"
    );
    let _ = std::fs::remove_dir_all(root);
}
