//! TASK-0140: black-box proof for finite-job execution timeouts.
//!
//! Real watcher, real process trees, real control socket. A job whose
//! configured `timeout:` elapses must terminate its whole process tree and
//! reach the typed terminal timeout outcome — distinct from ordinary
//! failures, from user cancellation, and from the client await deadline.
//! Assertions synchronize on observable state (log lines, socket replies,
//! process existence); outer loops are generous safety bounds only.

#![cfg(all(feature = "test-integration", unix))]

use std::io::{BufRead, BufReader};
use std::os::unix::fs::PermissionsExt;
use std::os::unix::net::UnixStream;
use std::process::{Child, Command, Output, Stdio};
use std::time::{Duration, Instant};

struct TestProcess {
    child: Child,
    directory: std::path::PathBuf,
}

impl Drop for TestProcess {
    fn drop(&mut self) {
        // Graceful watcher shutdown lets it clean up any process groups it
        // owns; direct Child::kill would leave descendants orphaned.
        let pid = self.child.id().to_string();
        let _ = Command::new("kill").args(["-INT", &pid]).status();
        let deadline = Instant::now() + Duration::from_secs(2);
        loop {
            match self.child.try_wait() {
                Ok(Some(_)) => break,
                Ok(None) if Instant::now() < deadline => {
                    std::thread::sleep(Duration::from_millis(20));
                }
                _ => {
                    let _ = self.child.kill();
                    let _ = self.child.wait();
                    break;
                }
            }
        }
        let _ = std::fs::remove_dir_all(&self.directory);
    }
}

static DIRECTORY_COUNTER: std::sync::atomic::AtomicU32 = std::sync::atomic::AtomicU32::new(0);

fn setup_directory(test_name: &str, config: &str) -> std::path::PathBuf {
    let counter = DIRECTORY_COUNTER.fetch_add(1, std::sync::atomic::Ordering::SeqCst);
    let directory = std::env::temp_dir().join(format!(
        "funzzy-timeout-proof-{}-{test_name}-{counter}",
        std::process::id()
    ));
    let _ = std::fs::remove_dir_all(&directory);
    std::fs::create_dir_all(directory.join("src")).unwrap();
    std::fs::write(directory.join(".watch.yaml"), config).unwrap();
    // Blocking job with a live descendant: prints attributable evidence,
    // records both PIDs, then blocks forever. The whole tree must be
    // terminated when the deadline elapses.
    std::fs::write(
        directory.join("tree.sh"),
        r#"#!/bin/sh
printf 'pre-kill-evidence\n'
echo $$ > tree.pid
sh -c 'echo $$ > descendant.pid; while true; do sleep 0.05; done' &
while true; do sleep 0.05; done
"#,
    )
    .unwrap();
    // TERM-resistant parent and descendant: timeout must wait through the
    // configured grace period and escalate to SIGKILL before reaping both.
    std::fs::write(
        directory.join("stubborn.sh"),
        r#"#!/bin/sh
trap 'printf parent-term-ignored >> parent-term-ignored' TERM INT
echo $$ > stubborn.pid
sh -c 'trap "printf descendant-term-ignored >> descendant-term-ignored" TERM INT; echo $$ > stubborn-descendant.pid; while true; do sleep 0.05; done' &
while true; do sleep 0.05; done
"#,
    )
    .unwrap();
    for script in ["tree.sh", "stubborn.sh"] {
        let mut permissions = std::fs::metadata(directory.join(script))
            .unwrap()
            .permissions();
        permissions.set_mode(0o755);
        std::fs::set_permissions(directory.join(script), permissions).unwrap();
    }
    std::fs::canonicalize(directory).unwrap()
}

fn start_watcher(directory: &std::path::Path) -> TestProcess {
    let child_log = std::fs::File::create(directory.join("watcher.log")).unwrap();
    let child = Command::new(env!("CARGO_BIN_EXE_fzz"))
        .current_dir(directory)
        .env_remove("FUNZZY_BAIL")
        .env_remove("FUNZZY_NON_BLOCK")
        .env("FUNZZY_COLORED", "false")
        .env("FUNZZY_CANCEL_GRACE_MS", "100")
        .stdout(Stdio::from(child_log.try_clone().unwrap()))
        .stderr(Stdio::from(child_log))
        .spawn()
        .unwrap();
    TestProcess {
        child,
        directory: directory.to_path_buf(),
    }
}

fn wait_until<F: FnMut() -> bool>(mut condition: F, description: &str) {
    let deadline = Instant::now() + Duration::from_secs(20);
    loop {
        if condition() {
            return;
        }
        if Instant::now() > deadline {
            panic!("timed out waiting for {description}");
        }
        std::thread::sleep(Duration::from_millis(20));
    }
}

fn wait_until_socket(directory: &std::path::Path) {
    wait_until(
        || UnixStream::connect(directory.join("sock")).is_ok(),
        "control socket",
    );
}

fn run_cli(directory: &std::path::Path, args: &[&str]) -> Output {
    Command::new(env!("CARGO_BIN_EXE_fzz"))
        .current_dir(directory)
        .env("FUNZZY_COLORED", "false")
        .env("FUNZZY_CANCEL_GRACE_MS", "100")
        .args(args)
        .output()
        .expect("fzz command should run")
}

fn combined(output: &Output) -> String {
    format!(
        "{}{}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    )
}

fn scheduled_generation(output: &Output) -> u64 {
    let text = combined(output);
    text.lines()
        .find_map(|line| line.strip_prefix("scheduled generation: "))
        .unwrap_or_else(|| panic!("missing generation in {text}"))
        .parse()
        .expect("generation is numeric")
}

fn status(socket: &std::path::Path) -> serde_json::Value {
    let mut stream = UnixStream::connect(socket).expect("connect control socket");
    std::io::Write::write_all(
        &mut stream,
        br#"{"jsonrpc":"2.0","id":"status","method":"status"}
"#,
    )
    .unwrap();
    let mut line = String::new();
    BufReader::new(stream).read_line(&mut line).unwrap();
    serde_json::from_str(&line).unwrap()
}

/// True while the recorded PID still exists (`kill 0` probe).
fn process_alive(pid: u32) -> bool {
    // Safe existence probe: signal 0 never delivers anything.
    Command::new("kill")
        .args(["-0", &pid.to_string()])
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .status()
        .map(|status| status.success())
        .unwrap_or(false)
}

fn await_generation(directory: &std::path::Path, generation: u64, format: &[&str]) -> Output {
    let mut args: Vec<&str> = vec!["control"];
    args.extend_from_slice(format);
    let generation_arg = generation.to_string();
    args.extend_from_slice(&["await", "--generation", &generation_arg, "--timeout", "30s"]);
    run_cli(directory, &args)
}

fn await_json(directory: &std::path::Path, generation: u64) -> serde_json::Value {
    let output = await_generation(directory, generation, &["--format", "json"]);
    let text = combined(&output);
    // The JSON observation document comes first; a failed/cancelled
    // terminal outcome then exits non-zero with an Error trailer (by
    // contract). Parse the document regardless of the exit status.
    let payload = text
        .find("Error:")
        .map(|end| &text[..end])
        .unwrap_or(text.trim_end());
    let mut stream =
        serde_json::Deserializer::from_str(payload.trim()).into_iter::<serde_json::Value>();
    stream
        .next()
        .unwrap_or_else(|| panic!("invalid await JSON: {text}"))
        .unwrap_or_else(|err| panic!("invalid await JSON {err}: {text}"))
}

const TIMED_TREE: &str = r#"
on:
  socket: sock
jobs:
  - name: tree
    run: ./tree.sh
    change: 'src/**'
    timeout: 1s
"#;

const STUBBORN_TREE: &str = r#"
on:
  socket: sock
jobs:
  - name: stubborn
    run: ./stubborn.sh
    change: 'src/**'
    timeout: 500ms
"#;

#[test]
fn timeout_escalates_term_ignoring_process_tree_and_reaps_it() {
    let directory = setup_directory("escalation", STUBBORN_TREE);
    let _watcher = start_watcher(&directory);
    wait_until_socket(&directory);

    let run = run_cli(&directory, &["control", "run", "stubborn"]);
    let generation = scheduled_generation(&run);
    wait_until(
        || {
            directory.join("stubborn.pid").exists()
                && directory.join("stubborn-descendant.pid").exists()
        },
        "TERM-resistant tree recorded both pids",
    );

    let observation = await_json(&directory, generation);
    assert_eq!(observation["terminalReason"], "failed");
    assert!(
        observation["snapshot"]["tasks"]
            .as_array()
            .is_some_and(|tasks| tasks.iter().any(|task| task["state"] == "timedout")),
        "timeout must remain typed after escalation: {observation}"
    );

    // Both handlers observed SIGTERM and deliberately stayed alive. Since
    // the test watcher uses a 100ms grace, terminal completion proves the
    // shutdown path escalated to SIGKILL rather than merely sending TERM.
    wait_until(
        || {
            directory.join("parent-term-ignored").exists()
                && directory.join("descendant-term-ignored").exists()
        },
        "TERM-resistant handlers observed SIGTERM",
    );
    let parent_pid: u32 = std::fs::read_to_string(directory.join("stubborn.pid"))
        .unwrap()
        .trim()
        .parse()
        .expect("stubborn pid");
    let descendant_pid: u32 = std::fs::read_to_string(directory.join("stubborn-descendant.pid"))
        .unwrap()
        .trim()
        .parse()
        .expect("stubborn descendant pid");
    wait_until(
        || !process_alive(parent_pid) && !process_alive(descendant_pid),
        "SIGKILL-escalated process tree reaped",
    );
}

#[test]
fn timeout_terminates_the_process_tree_with_typed_outcome_and_evidence() {
    let directory = setup_directory("tree", TIMED_TREE);
    let _watcher = start_watcher(&directory);
    wait_until_socket(&directory);

    let run = run_cli(&directory, &["control", "run", "tree"]);
    let generation = scheduled_generation(&run);
    wait_until(
        || directory.join("tree.pid").exists() && directory.join("descendant.pid").exists(),
        "tree script recorded its pids before timeout",
    );

    let observation = await_json(&directory, generation);
    assert_eq!(
        observation["terminalReason"], "failed",
        "a timed-out job fails the generation: {observation}"
    );
    let failures = observation["snapshot"]["failures"]
        .as_array()
        .cloned()
        .unwrap_or_default();
    assert!(
        failures
            .iter()
            .any(|f| f.as_str().is_some_and(|s| s.contains("timed out"))),
        "failure evidence must name the timeout: {observation}"
    );
    // Typed task state on the structured surface.
    let states: Vec<&str> = observation["snapshot"]["tasks"]
        .as_array()
        .map(|tasks| {
            tasks
                .iter()
                .map(|task| task["state"].as_str().unwrap_or_default())
                .collect()
        })
        .unwrap_or_default();
    assert!(
        states.contains(&"timedout"),
        "task snapshot must carry the additive timedout state: {observation}"
    );

    // The whole process tree is reaped: parent and descendant PIDs recorded
    // by the script are gone once the terminal outcome is observed.
    wait_until(
        || directory.join("tree.pid").exists() && directory.join("descendant.pid").exists(),
        "tree script recorded its pids",
    );
    let parent_pid: u32 = std::fs::read_to_string(directory.join("tree.pid"))
        .unwrap()
        .trim()
        .parse()
        .expect("tree pid");
    let descendant_pid: u32 = std::fs::read_to_string(directory.join("descendant.pid"))
        .unwrap()
        .trim()
        .parse()
        .expect("descendant pid");
    wait_until(
        || !process_alive(parent_pid) && !process_alive(descendant_pid),
        "timed-out process tree reaped",
    );

    // Pre-kill output stays bounded, attributable, and retrievable by the
    // exact generation.
    let evidence = run_cli(
        &directory,
        &["control", "output", "--generation", &generation.to_string()],
    );
    assert!(
        combined(&evidence).contains("pre-kill-evidence"),
        "pre-kill evidence must be retrievable: {}",
        combined(&evidence)
    );

    // TOON agrees on the typed state (agent-facing surface).
    let toon = await_generation(&directory, generation, &["--format", "toon"]);
    assert!(
        combined(&toon).contains("timedout"),
        "toon output must carry the timedout state: {}",
        combined(&toon)
    );
}

#[test]
fn natural_success_and_failure_before_deadline_keep_ordinary_outcomes() {
    let directory = setup_directory(
        "natural",
        r#"
on:
  socket: sock
jobs:
  - name: quick-ok @mixed
    run: 'printf natural-passed'
    change: 'src/**'
    timeout: 60s
  - name: quick-fail @mixed
    run: 'printf natural-failed >&2; exit 3'
    change: 'src/**'
    timeout: 60s
"#,
    );
    let _watcher = start_watcher(&directory);
    wait_until_socket(&directory);

    std::fs::write(directory.join("src/change.txt"), "touch").unwrap();
    let run = run_cli(&directory, &["control", "run", "@mixed"]);
    let generation = scheduled_generation(&run);

    let observation = await_json(&directory, generation);
    assert_eq!(
        observation["terminalReason"], "failed",
        "the exit-3 job fails the generation ordinarily: {observation}"
    );
    let failures = observation["snapshot"]["failures"]
        .as_array()
        .cloned()
        .unwrap_or_default();
    assert!(
        failures
            .iter()
            .all(|f| f.as_str().is_some_and(|s| !s.contains("timed out"))),
        "no timeout may be reported before the deadline: {observation}"
    );
    let states: Vec<String> = observation["snapshot"]["tasks"]
        .as_array()
        .map(|tasks| {
            tasks
                .iter()
                .map(|task| task["state"].as_str().unwrap_or_default().to_owned())
                .collect()
        })
        .unwrap_or_default();
    assert!(states.contains(&"passed".to_owned()), "{observation}");
    assert!(states.contains(&"failed".to_owned()), "{observation}");
}

#[test]
fn cancellation_before_deadline_wins_and_never_becomes_timeout() {
    let directory = setup_directory(
        "cancel",
        r#"
on:
  socket: sock
jobs:
  - name: tree
    run: ./tree.sh
    change: 'src/**'
    timeout: 60s
"#,
    );
    let _watcher = start_watcher(&directory);
    wait_until_socket(&directory);
    let socket = directory.join("sock");

    std::fs::write(directory.join("src/change.txt"), "touch").unwrap();
    let run = run_cli(&directory, &["control", "run", "tree"]);
    let generation = scheduled_generation(&run);
    wait_until(
        || {
            let current = status(&socket);
            current["result"]["generation"].as_u64() == Some(generation)
                && current["result"]["state"].as_str() == Some("running")
        },
        "job running before cancellation",
    );

    let cancel = run_cli(
        &directory,
        &[
            "control",
            "cancel",
            "--generation",
            &generation.to_string(),
            "--wait",
            "--timeout",
            "30s",
        ],
    );
    assert!(
        combined(&cancel).contains("terminal reason: cancelled"),
        "cancel: {}",
        combined(&cancel)
    );

    // The terminal outcome is immutable: awaiting the same generation again
    // still reports cancelled — the generous timeout never converts it.
    let observation = await_json(&directory, generation);
    assert_eq!(
        observation["terminalReason"], "cancelled",
        "cancellation must stay terminal: {observation}"
    );
}

#[test]
fn client_await_timeout_does_not_cancel_running_generation() {
    let directory = setup_directory(
        "client-wait",
        r#"
on:
  socket: sock
jobs:
  - name: tree
    run: ./tree.sh
    change: 'src/**'
    timeout: 60s
"#,
    );
    let _watcher = start_watcher(&directory);
    wait_until_socket(&directory);
    let socket = directory.join("sock");

    let run = run_cli(&directory, &["control", "run", "tree"]);
    let generation = scheduled_generation(&run);
    wait_until(
        || directory.join("tree.pid").exists() && directory.join("descendant.pid").exists(),
        "long-running tree recorded both pids",
    );

    // A short control wait reports timeout, but has no authority to cancel
    // the generation or its child process tree.
    let waited = run_cli(
        &directory,
        &[
            "control",
            "await",
            "--generation",
            &generation.to_string(),
            "--timeout",
            "100ms",
        ],
    );
    assert_eq!(waited.status.code(), Some(1), "await should time out");
    assert!(
        combined(&waited).contains("terminal reason: timeout"),
        "await timeout output: {}",
        combined(&waited)
    );
    let current = status(&socket);
    assert_eq!(
        current["result"]["generation"].as_u64(),
        Some(generation),
        "short await must leave generation identity unchanged: {current}"
    );
    assert_eq!(
        current["result"]["state"].as_str(),
        Some("running"),
        "short await must not cancel the running generation: {current}"
    );

    // Explicit cancellation remains responsible for ending and reaping it.
    let cancel = run_cli(
        &directory,
        &[
            "control",
            "cancel",
            "--generation",
            &generation.to_string(),
            "--wait",
            "--timeout",
            "30s",
        ],
    );
    assert!(
        combined(&cancel).contains("terminal reason: cancelled"),
        "explicit cancel: {}",
        combined(&cancel)
    );
    let parent_pid: u32 = std::fs::read_to_string(directory.join("tree.pid"))
        .unwrap()
        .trim()
        .parse()
        .expect("tree pid");
    let descendant_pid: u32 = std::fs::read_to_string(directory.join("descendant.pid"))
        .unwrap()
        .trim()
        .parse()
        .expect("descendant pid");
    wait_until(
        || !process_alive(parent_pid) && !process_alive(descendant_pid),
        "explicit cancellation reaped the process tree",
    );
}

#[test]
fn reload_changes_timeout_only_for_later_generations() {
    let directory = setup_directory("reload", TIMED_TREE);
    let _watcher = start_watcher(&directory);
    wait_until_socket(&directory);
    let socket = directory.join("sock");

    // Generation 1 runs under timeout 1s and times out (single
    // deterministic trigger: exact control selection, no event batches).
    let first = run_cli(&directory, &["control", "run", "tree"]);
    let first_generation = scheduled_generation(&first);
    let first_observation = await_json(&directory, first_generation);
    assert_eq!(first_observation["terminalReason"], "failed");

    // Reload a generous timeout: revision 2 becomes live.
    std::fs::write(
        directory.join(".watch.yaml"),
        TIMED_TREE.replace("1s", "60s"),
    )
    .unwrap();
    wait_until(
        || {
            std::fs::read_to_string(directory.join("watcher.log"))
                .unwrap_or_default()
                .contains("hot-reloading to revision 2")
        },
        "timeout-only reload commits revision 2",
    );

    // Generation 2 uses the new deadline: a plain path event (same shape as
    // generation 1) schedules the replacement work under revision 2.
    std::fs::write(directory.join("src/again.txt"), "touch").unwrap();
    let second_generation = first_generation + 1;
    wait_until(
        || {
            let current = status(&socket);
            current["result"]["generation"].as_u64() == Some(second_generation)
                && current["result"]["state"].as_str() == Some("running")
        },
        "replacement generation running under the reloaded timeout",
    );
    // Far beyond the old 1s budget (plus shutdown grace), the replacement
    // generation must STILL be running: the stale timer never fires on it.
    let survival_deadline = Instant::now() + Duration::from_millis(1_500);
    while Instant::now() < survival_deadline {
        let current = status(&socket);
        assert_eq!(
            current["result"]["generation"].as_u64(),
            Some(second_generation),
            "replacement generation identity changed: {current}"
        );
        assert_eq!(
            current["result"]["state"].as_str(),
            Some("running"),
            "old deadline must not terminate replacement work: {current}"
        );
        std::thread::sleep(Duration::from_millis(50));
    }

    let cancel = run_cli(
        &directory,
        &[
            "control",
            "cancel",
            "--generation",
            &second_generation.to_string(),
            "--wait",
            "--timeout",
            "30s",
        ],
    );
    assert!(
        combined(&cancel).contains("terminal reason: cancelled"),
        "second generation ends by explicit cancellation: {}",
        combined(&cancel)
    );
}

#[test]
fn local_run_agrees_on_timeout_semantics() {
    let directory = setup_directory("local", TIMED_TREE);

    let local = run_cli(&directory, &["run", "tree"]);
    let local_text = combined(&local);
    assert!(
        !local.status.success(),
        "a timed-out local run must exit non-zero: {local_text}"
    );
    assert!(
        local_text.contains("timed out"),
        "local output must name the timeout: {local_text}"
    );

    // The process tree is reaped locally too.
    let descendant_pid: u32 = std::fs::read_to_string(directory.join("descendant.pid"))
        .unwrap()
        .trim()
        .parse()
        .expect("descendant pid");
    wait_until(
        || !process_alive(descendant_pid),
        "local timed-out descendant reaped",
    );
}
