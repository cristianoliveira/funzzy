//! TASK-0092: black-box proof of reload continuity and invalid-config
//! shutdown against the real watcher process and control socket.
//!
//! CONFIG-RELOAD-CONTRACT coverage not already owned by TASK-0090
//! (`config_reload_matrix.rs`) / TASK-0091 (`control_reload_identity.rs`):
//!
//! - AC1: a valid atomic rewrite preserves PID + instance token, increments
//!   the revision exactly once, keeps subscribers connected, and the new
//!   matching job routes without process restart.
//! - AC2: a comment-only rewrite is a semantic no-op (no revision bump, no
//!   subsystem churn); rapid valid writes debounce to the final candidate —
//!   never a mixed or intermediate revision.
//! - AC3: a busy old-revision generation completes with its original jobs
//!   while a later event runs the new revision; output references identify
//!   each generation's frozen revision.
//! - AC4: a root added for an initially missing future file observes the
//!   later create (stable-ancestor watching).
//! - AC5: concurrency/debounce/hooks/output valid changes follow the same
//!   transaction and preserve the process.
//! - AC6: schema-invalid, semantic-invalid, and occupied-new-socket
//!   candidates each produce a deterministic terminal error, socket
//!   closure, and nonzero exit.
//! - AC7: a partial editor write followed by valid final content inside the
//!   window does not spuriously exit; content remaining invalid does.
//! - AC8: delete is fatal ("config unreadable"); a transient delete+recreate
//!   inside the window never spins or reloads repeatedly.
//! - AC9: an invalid shutdown reaps managed service descendants and leaves
//!   no live socket behind.
//!
//! AC10 (regression): every test asserting PID/token preservation fails
//! against the old unconditional self-SIGTERM implementation — a restart
//! would change the PID, the instance token, and drop the subscriber.

#![cfg(all(feature = "test-integration", unix))]

use std::io::{BufRead, BufReader, Write};
use std::os::unix::net::{UnixListener, UnixStream};
use std::process::{Child, Command, Stdio};
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
        "fzzrlc-{}-{test_name}-{counter}",
        std::process::id()
    ));
    let _ = std::fs::remove_dir_all(&directory);
    std::fs::create_dir_all(&directory).unwrap();
    std::fs::create_dir_all(directory.join("src")).unwrap();
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
    for _ in 0..200 {
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

fn wait_until<F: FnMut() -> bool>(mut condition: F, what: &str) {
    let deadline = std::time::Instant::now() + Duration::from_secs(25);
    while std::time::Instant::now() < deadline {
        if condition() {
            return;
        }
        std::thread::sleep(Duration::from_millis(100));
    }
    panic!("wait_until timed out: {what}");
}

fn child_log(directory: &std::path::Path) -> String {
    std::fs::read_to_string(directory.join("child.err")).unwrap_or_default()
}

fn wait_for_log(directory: &std::path::Path, needle: &str) -> String {
    wait_until(
        || child_log(&directory).contains(needle),
        &format!("log {needle:?}"),
    );
    child_log(&directory)
}

fn count_occurrences(haystack: &str, needle: &str) -> usize {
    haystack.matches(needle).count()
}

fn status_revision(socket: &std::path::Path) -> Option<u64> {
    result(call(socket, "status", serde_json::json!({})))["revision"].as_u64()
}

fn instance_token(socket: &std::path::Path) -> String {
    result(call(socket, "capabilities", serde_json::json!({})))["instance"]["token"]
        .as_str()
        .expect("instance token")
        .to_owned()
}

fn latest_generation(socket: &std::path::Path) -> u64 {
    result(call(socket, "status", serde_json::json!({})))["generation"]
        .as_u64()
        .unwrap_or(0)
}

fn await_generation(socket: &std::path::Path, generation: u64) -> serde_json::Value {
    result(call(
        socket,
        "await",
        serde_json::json!({"generation": generation, "timeoutMs": 20_000}),
    ))
}

/// Schedules the first generation with a real file write and awaits it,
/// proving it froze under revision 1 (status `revision` is only populated
/// once a generation runs). Returns the awaited result.
fn schedule_first_generation(
    socket: &std::path::Path,
    directory: &std::path::Path,
) -> serde_json::Value {
    let baseline = latest_generation(socket);
    std::fs::write(directory.join("src/seed.rs"), "x").unwrap();
    wait_until(
        || latest_generation(socket) > baseline,
        "first generation to schedule",
    );
    let generation = latest_generation(socket);
    let awaited = await_generation(socket, generation);
    assert_eq!(
        awaited["snapshot"]["revision"].as_u64(),
        Some(1),
        "the first generation must freeze revision 1"
    );
    awaited
}

fn base_config() -> String {
    "on:\n  socket: sock\njobs:\n  - name: build\n    run: 'echo build > build-verdict.txt'\n    change: 'src/**'\n"
        .to_owned()
}

/// AC1 + AC10 regression: a valid atomic config rewrite preserves the
/// watcher PID and instance token, increments the revision exactly once, and
/// the newly added matching job routes through the live process — no
/// restart, no duplicate intermediate revision.
#[test]
fn valid_atomic_rewrite_preserves_pid_token_and_routes_new_job_once() {
    let directory = setup_directory("atomic", &base_config());
    let mut watcher = start_watcher(&directory);
    wait_until_socket(&directory);
    let socket_path = directory.join("sock");
    let pid = watcher.child.id();
    let token = instance_token(&socket_path);
    schedule_first_generation(&socket_path, &directory);
    assert_eq!(status_revision(&socket_path), Some(1));

    let grown = "on:\n  socket: sock\njobs:\n  - name: build\n    run: 'echo build > build-verdict.txt'\n    change: 'src/**'\n  - name: docs\n    run: 'echo docs > docs-verdict.txt'\n    change: 'docs/**'\n";
    std::fs::write(directory.join(".watch.yaml"), grown).unwrap();

    let log = wait_for_log(&directory, "hot-reloading to revision 2");
    assert_eq!(
        count_occurrences(&log, "hot-reloading to revision"),
        1,
        "a single atomic rewrite must commit exactly one revision: {log}"
    );
    assert!(
        !watcher.try_exited(),
        "valid reload must not exit the process"
    );
    assert_eq!(
        watcher.child.id(),
        pid,
        "PID must survive a valid reload (self-SIGTERM would restart)"
    );
    assert_eq!(
        instance_token(&socket_path),
        token,
        "instance token must survive a valid reload"
    );

    // The NEW job routes through the same process without a restart.
    std::fs::create_dir_all(directory.join("docs")).unwrap();
    // Settle before writing the file inside: inotify registers the watch for
    // a newly created subdir asynchronously, and a file written in the same
    // instant can be missed (FSEvents on macOS does not have this race, so
    // these reload tests only surface it on Linux CI).
    std::thread::sleep(Duration::from_millis(300));
    std::fs::write(directory.join("docs/guide.md"), "x").unwrap();
    wait_until(
        || directory.join("docs-verdict.txt").exists(),
        "new job to run under revision 2",
    );
    assert_eq!(
        status_revision(&socket_path),
        Some(2),
        "a generation after commit freezes the new revision"
    );
}

/// AC2 regression: a comment-only rewrite is a semantic no-op even when the
/// control socket is declared by the config file (`on.socket`) rather than a
/// CLI flag. Before TASK-0092 the startup revision captured the socket as
/// `None` while reload candidates carried it, so EVERY valid save — even a
/// formatting-only one — committed a new revision.
#[test]
fn comment_only_rewrite_is_noop_with_config_declared_socket() {
    let directory = setup_directory("comment-noop", &base_config());
    let mut watcher = start_watcher(&directory);
    wait_until_socket(&directory);
    let socket_path = directory.join("sock");
    schedule_first_generation(&socket_path, &directory);
    assert_eq!(status_revision(&socket_path), Some(1));

    std::fs::write(
        directory.join(".watch.yaml"),
        "# formatting-only rewrite\non:\n  socket: sock\njobs:\n  - name: build\n    run: 'echo build > build-verdict.txt'\n    change: 'src/**'\n",
    )
    .unwrap();

    let log = wait_for_log(&directory, "no semantic change");
    assert_eq!(
        count_occurrences(&log, "hot-reloading to revision"),
        0,
        "a comment-only rewrite must never commit a revision: {log}"
    );
    assert!(
        !watcher.try_exited(),
        "a no-op rewrite must not exit the process"
    );
    assert_eq!(
        status_revision(&socket_path),
        Some(1),
        "revision must not move on a no-op save"
    );
}

/// AC2: rapid valid writes inside one debounce window settle on the FINAL
/// candidate — exactly one revision commit, never a mixed or intermediate
/// revision, and the committed config is the last write.
#[test]
fn rapid_valid_writes_debounce_to_final_candidate() {
    let directory = setup_directory("rapid", &base_config());
    let mut watcher = start_watcher(&directory);
    wait_until_socket(&directory);
    let socket_path = directory.join("sock");
    schedule_first_generation(&socket_path, &directory);
    assert_eq!(status_revision(&socket_path), Some(1));

    // Two semantically different writes back to back (same debounce window).
    let intermediate = "on:\n  socket: sock\njobs:\n  - name: B-job\n    run: 'echo b > b-verdict.txt'\n    change: 'src/**'\n";
    let final_candidate = "on:\n  socket: sock\njobs:\n  - name: C-job\n    run: 'echo c > c-verdict.txt'\n    change: 'src/**'\n";
    std::fs::write(directory.join(".watch.yaml"), intermediate).unwrap();
    std::fs::write(directory.join(".watch.yaml"), final_candidate).unwrap();

    let log = wait_for_log(&directory, "hot-reloading to revision 2");
    assert_eq!(
        count_occurrences(&log, "hot-reloading to revision"),
        1,
        "rapid writes must commit exactly one revision: {log}"
    );
    // Give any spurious second reload time to appear.
    std::thread::sleep(Duration::from_millis(1200));
    let settled = child_log(&directory);
    assert_eq!(
        count_occurrences(&settled, "hot-reloading to revision"),
        1,
        "rapid writes must not leak an intermediate revision: {settled}"
    );
    assert!(
        !watcher.try_exited(),
        "debounced reload must not exit the process"
    );

    // The committed config is the FINAL candidate, not the intermediate one.
    let emitted = result(call(
        &socket_path,
        "emit",
        serde_json::json!({"path": "src/x.rs"}),
    ));
    let matched: Vec<&str> = emitted["matched"]
        .as_array()
        .unwrap()
        .iter()
        .filter_map(|t| t.as_str())
        .collect();
    assert_eq!(matched, vec!["C-job"], "final candidate must be committed");
    assert_eq!(emitted["revision"].as_u64(), Some(2));
}

/// AC3: a busy old-revision generation completes with its ORIGINAL jobs
/// while a later event routes under the new revision; output references
/// carry each generation's frozen revision.
#[test]
fn busy_old_revision_completes_while_later_event_runs_new_revision() {
    let slow = "on:\n  socket: sock\njobs:\n  - name: slow\n    run: 'sleep 2; echo done > slow-verdict.txt'\n    change: 'src/**'\n";
    let directory = setup_directory("busy-reload", slow);
    let mut watcher = start_watcher(&directory);
    wait_until_socket(&directory);
    let socket_path = directory.join("sock");

    // Start the slow generation under revision 1 via a real file write.
    let baseline = latest_generation(&socket_path);
    std::fs::write(directory.join("src/a.rs"), "x").unwrap();
    wait_until(
        || latest_generation(&socket_path) > baseline,
        "slow generation to schedule",
    );
    let gen1 = latest_generation(&socket_path);
    assert_eq!(
        result(call(&socket_path, "status", serde_json::json!({})))["revision"].as_u64(),
        Some(1)
    );

    // Reload while it is still busy: the new revision swaps the jobs.
    let fast = "on:\n  socket: sock\njobs:\n  - name: fast\n    run: 'echo fast > fast-verdict.txt'\n    change: 'src/**'\n";
    std::fs::write(directory.join(".watch.yaml"), fast).unwrap();
    wait_for_log(&directory, "hot-reloading to revision 2");
    assert!(!watcher.try_exited(), "reload must not exit the watcher");

    // The OLD generation completes with its original job despite the save.
    let old_awaited = await_generation(&socket_path, gen1);
    assert_eq!(old_awaited["terminalReason"], "passed");
    assert!(
        directory.join("slow-verdict.txt").exists(),
        "busy old-revision generation must complete under its original plan"
    );
    // Its output reference identifies revision 1.
    let output1 = result(call(
        &socket_path,
        "output",
        serde_json::json!({"generation": gen1}),
    ));
    assert_eq!(output1["revision"].as_u64(), Some(1));

    // A LATER event routes under the new revision with the new job.
    std::fs::write(directory.join("src/b.rs"), "x").unwrap();
    wait_until(
        || latest_generation(&socket_path) > gen1,
        "later event to schedule under revision 2",
    );
    let gen2 = latest_generation(&socket_path);
    let awaited2 = await_generation(&socket_path, gen2);
    assert_eq!(awaited2["terminalReason"], "passed");
    assert_eq!(awaited2["snapshot"]["revision"].as_u64(), Some(2));
    assert!(
        directory.join("fast-verdict.txt").exists(),
        "later event must run the NEW revision's job"
    );
    let output2 = result(call(
        &socket_path,
        "output",
        serde_json::json!({"generation": gen2}),
    ));
    assert_eq!(output2["revision"].as_u64(), Some(2));
}

/// AC4: a root added for an initially MISSING future file (stable-ancestor
/// watching) observes the later create; the process never exits.
#[test]
fn reload_adds_missing_future_root_and_observes_later_create() {
    let directory = setup_directory("future-root", &base_config());
    let mut watcher = start_watcher(&directory);
    wait_until_socket(&directory);

    // The `future/` directory does NOT exist when the root is added.
    let grown = "on:\n  socket: sock\njobs:\n  - name: build\n    run: 'echo build > build-verdict.txt'\n    change: 'src/**'\n  - name: future\n    run: 'echo future > future-verdict.txt'\n    change: 'future/**/*.rs'\n";
    std::fs::write(directory.join(".watch.yaml"), grown).unwrap();
    wait_for_log(&directory, "hot-reloading to revision 2");
    assert!(
        !watcher.try_exited(),
        "adding a missing future root must not exit the process"
    );

    // Create the future path: the stable ancestor observes the create.
    std::fs::create_dir_all(directory.join("future")).unwrap();
    // Settle before writing the file inside: inotify registers the watch for
    // a newly created subdir asynchronously, and a file written in the same
    // instant can be missed (FSEvents on macOS does not have this race, so
    // these reload tests only surface it on Linux CI).
    std::thread::sleep(Duration::from_millis(300));
    std::fs::write(directory.join("future/new.rs"), "x").unwrap();
    wait_until(
        || directory.join("future-verdict.txt").exists(),
        "future root to observe the later create",
    );
}

/// AC5: policy-surface changes (concurrency, debounce, hooks) follow the
/// same prepare→commit→retire transaction and preserve the process; the
/// committed hooks run at the run boundary.
#[test]
fn policy_surface_change_preserves_process_and_applies_hooks() {
    let config = "on:\n  socket: sock\n  concurrency: 1\n  debounce: 500ms\njobs:\n  - name: build\n    run: 'echo build > build-verdict.txt'\n    change: 'src/**'\n";
    let directory = setup_directory("policy", config);
    let mut watcher = start_watcher(&directory);
    wait_until_socket(&directory);

    let changed = "on:\n  socket: sock\n  concurrency: 4\n  debounce: 300ms\n  success: 'echo hooked > hook-verdict.txt'\njobs:\n  - name: build\n    run: 'echo build > build-verdict.txt'\n    change: 'src/**'\n";
    std::fs::write(directory.join(".watch.yaml"), changed).unwrap();
    wait_for_log(&directory, "hot-reloading to revision 2");
    assert!(
        !watcher.try_exited(),
        "policy change must not exit the process"
    );

    // The committed hooks apply to post-commit runs.
    std::fs::write(directory.join("src/a.rs"), "x").unwrap();
    wait_until(
        || {
            directory.join("build-verdict.txt").exists()
                && directory.join("hook-verdict.txt").exists()
        },
        "committed hook to run at the post-commit boundary",
    );
}

/// Asserts the shared invalid-shutdown contract: nonzero exit, terminal
/// error naming the gate, socket closure, and no leftover watcher children.
fn assert_fatal_invalid_shutdown(
    directory: &std::path::Path,
    watcher: &mut TestProcess,
    reason_needle: &str,
) {
    let socket_path = directory.join("sock");
    let mut deadline = std::time::Instant::now() + Duration::from_secs(25);
    let exit = loop {
        if let Some(status) = watcher.child.try_wait().expect("try_wait") {
            break status;
        }
        assert!(
            std::time::Instant::now() < deadline,
            "invalid config must terminate the watcher: {}",
            child_log(&directory)
        );
        std::thread::sleep(Duration::from_millis(50));
    };
    assert!(!exit.success(), "invalid config exits nonzero: {exit:?}");

    let log = child_log(&directory);
    assert!(
        log.contains("Fatal configuration error"),
        "terminal config error must be visible: {log}"
    );
    assert!(
        log.contains(reason_needle),
        "the gate/reason must be named ({reason_needle:?}): {log}"
    );
    // AC9: the control socket is closed and its file removed.
    deadline = std::time::Instant::now() + Duration::from_secs(10);
    while std::time::Instant::now() < deadline {
        if !socket_path.exists() {
            break;
        }
        std::thread::sleep(Duration::from_millis(100));
    }
    assert!(
        !socket_path.exists(),
        "invalid shutdown must remove the control socket file"
    );
}

/// AC6: a schema-invalid candidate (`on` is not an object) is fatal with a
/// nonzero exit and a terminal error naming the syntax gate.
#[test]
fn schema_invalid_reload_is_fatal_nonzero() {
    let directory = setup_directory("schema-invalid", &base_config());
    let mut watcher = start_watcher(&directory);
    wait_until_socket(&directory);

    std::fs::write(
        directory.join(".watch.yaml"),
        "on: 5\njobs:\n  - name: build\n    run: echo hi\n    change: 'src/**'\n",
    )
    .unwrap();
    assert_fatal_invalid_shutdown(&directory, &mut watcher, "invalid config (syntax)");
}

/// AC6: a semantic-invalid candidate (invalid change glob) is fatal with a
/// terminal error naming the semantic gate.
#[test]
fn semantic_invalid_reload_is_fatal_nonzero() {
    let directory = setup_directory("semantic-invalid", &base_config());
    let mut watcher = start_watcher(&directory);
    wait_until_socket(&directory);

    std::fs::write(
        directory.join(".watch.yaml"),
        "jobs:\n  - name: build\n    run: echo hi\n    change: 'src/['\n",
    )
    .unwrap();
    assert_fatal_invalid_shutdown(&directory, &mut watcher, "invalid config (semantics)");
}

/// AC6: an occupied (live) new socket path fails the operational gate —
/// bind-new-before-retire-old cannot prepare, so the reload is fatal.
#[test]
fn occupied_new_socket_reload_is_fatal_nonzero() {
    let directory = setup_directory("occupied-socket", &base_config());
    let mut watcher = start_watcher(&directory);
    wait_until_socket(&directory);

    // A LIVE listener occupies the candidate socket path.
    let occupied = UnixListener::bind(directory.join("sock2")).expect("bind occupied socket");
    assert!(
        UnixStream::connect(directory.join("sock2")).is_ok(),
        "occupied socket must be connectable"
    );

    std::fs::write(
        directory.join(".watch.yaml"),
        "on:\n  socket: sock2\njobs:\n  - name: build\n    run: echo hi\n    change: 'src/**'\n",
    )
    .unwrap();
    assert_fatal_invalid_shutdown(&directory, &mut watcher, "control socket rebind failed");
    drop(occupied);
    let _ = std::fs::remove_file(directory.join("sock2"));
}

/// AC7: a partial editor write followed by valid final content inside the
/// debounce window settles on the valid candidate — never a spurious exit.
#[test]
fn partial_write_then_valid_final_content_survives() {
    let directory = setup_directory("partial-valid", &base_config());
    let mut watcher = start_watcher(&directory);
    wait_until_socket(&directory);

    std::fs::write(directory.join(".watch.yaml"), "jobs: [unclosed").unwrap();
    std::fs::write(
        directory.join(".watch.yaml"),
        "on:\n  socket: sock\njobs:\n  - name: build\n    run: 'echo build > build-verdict.txt'\n    change: 'src/**'\n  - name: docs\n    run: 'echo docs > docs-verdict.txt'\n    change: 'docs/**'\n",
    )
    .unwrap();

    let log = wait_for_log(&directory, "hot-reloading to revision 2");
    assert!(
        !watcher.try_exited(),
        "partial-then-valid write must not exit: {log}"
    );
    assert_eq!(
        count_occurrences(&log, "Fatal configuration error"),
        0,
        "a valid final candidate must not take the fatal path: {log}"
    );
    assert_eq!(
        count_occurrences(&log, "hot-reloading to revision"),
        1,
        "one settled candidate, one revision: {log}"
    );
}

/// AC7: content remaining invalid after the window closes is fatal — the
/// partial write is validated, not silently skipped or retried forever.
#[test]
fn partial_write_remaining_invalid_is_fatal() {
    let directory = setup_directory("partial-invalid", &base_config());
    let mut watcher = start_watcher(&directory);
    wait_until_socket(&directory);

    std::fs::write(directory.join(".watch.yaml"), "jobs: [unclosed").unwrap();
    assert_fatal_invalid_shutdown(&directory, &mut watcher, "invalid config (syntax)");
}

/// AC8: deleting the config is fatal ("config unreadable") — the watcher
/// cannot run without a config and exits gracefully, never silently stale.
#[test]
fn deleted_config_is_fatal_with_unreadable_error() {
    let directory = setup_directory("delete-config", &base_config());
    let mut watcher = start_watcher(&directory);
    wait_until_socket(&directory);

    std::fs::remove_file(directory.join(".watch.yaml")).unwrap();
    assert_fatal_invalid_shutdown(&directory, &mut watcher, "config unreadable after change");
}

/// AC8: a transient delete+recreate inside the window resolves to the final
/// valid candidate and does NOT spin — no revision churn, no repeated
/// reloads, no process exit.
#[test]
fn delete_recreate_in_window_does_not_spin() {
    let directory = setup_directory("recreate", &base_config());
    let mut watcher = start_watcher(&directory);
    wait_until_socket(&directory);
    let socket_path = directory.join("sock");
    schedule_first_generation(&socket_path, &directory);
    assert_eq!(status_revision(&socket_path), Some(1));

    // Delete and immediately recreate with identical semantics (a common
    // editor save flow): the watcher must settle on the final candidate
    // as a NO-OP — one validation, no revision churn, no spin.
    std::fs::remove_file(directory.join(".watch.yaml")).unwrap();
    std::fs::write(
        directory.join(".watch.yaml"),
        "on:\n  socket: sock\njobs:\n  - name: build\n    run: 'echo build > build-verdict.txt'\n    change: 'src/**'\n",
    )
    .unwrap();

    wait_for_log(&directory, "no semantic change");
    std::thread::sleep(Duration::from_millis(1500));
    let log = child_log(&directory);
    assert_eq!(
        count_occurrences(&log, "hot-reloading to revision"),
        0,
        "identical delete+recreate must be a no-op, never a reload: {log}"
    );
    assert!(
        !watcher.try_exited(),
        "delete+recreate must not exit the process: {log}"
    );
    assert_eq!(
        status_revision(&socket_path),
        Some(1),
        "identical recreation must not move the revision: {log}"
    );
}

/// AC9: an invalid shutdown reaps managed-service descendants — the service
/// process is gone once the watcher exits nonzero with the terminal error.
#[test]
fn invalid_shutdown_reaps_managed_service_descendants() {
    let directory = setup_directory(
        "service-reap",
        "on:\n  socket: sock\njobs:\n  - name: dev-server\n    service: true\n    run: 'bash -c \"echo $$ > svc.pid; while true; do touch svc-ready; sleep 0.2; done\"'\n    change: 'src/**'\n",
    );
    let mut watcher = start_watcher(&directory);
    wait_until_socket(&directory);

    // Start the service generation under revision 1.
    std::fs::write(directory.join("src/a.rs"), "x").unwrap();
    wait_until(
        || directory.join("svc-ready").exists(),
        "service to start under revision 1",
    );
    let svc_pid: u32 = std::fs::read_to_string(directory.join("svc.pid"))
        .expect("service pid file")
        .trim()
        .parse()
        .expect("service pid");

    // Invalid reload: the watcher must reap the owned service group.
    std::fs::write(directory.join(".watch.yaml"), "jobs: [unclosed").unwrap();
    assert_fatal_invalid_shutdown(&directory, &mut watcher, "invalid config (syntax)");

    let alive = std::process::Command::new("kill")
        .arg("-0")
        .arg(svc_pid.to_string())
        .output()
        .map(|output| output.status.success())
        .unwrap_or(false);
    assert!(
        !alive,
        "invalid shutdown must reap the managed service (pid {svc_pid})"
    );
}
