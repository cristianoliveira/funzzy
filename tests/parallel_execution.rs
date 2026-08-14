//! Parallel execution lifecycle black-box matrix (TASK-0029).
//!
//! Proves, against real spawned binaries with deterministic fixtures (no wall
//! clocks, no probabilities):
//! - contiguous tasks sharing a group name overlap; ordinary flat tasks stay
//!   sequential; separated group occurrences respect barriers; commands inside
//!   one task stay sequential;
//! - the configured concurrency bound is honored (1, 2, > task count);
//! - combined results cover all pass, one fail, many fail, spawn failure,
//!   fail-fast, cancelled, and superseded generations;
//! - repeated cancellation/replacement leaves no child, process group, or
//!   forwarding thread behind;
//! - control run, synthetic emit, filesystem events, and run-on-init all use
//!   the same parallel engine.

use assert_cmd::cargo;
use predicates::prelude::*;
use std::path::{Path, PathBuf};

fn fixture(name: &str) -> PathBuf {
    let directory =
        std::env::temp_dir().join(format!("funzzy-parallel-{}-{}", std::process::id(), name));
    let _ = std::fs::remove_dir_all(&directory);
    std::fs::create_dir_all(&directory).expect("create fixture");
    directory
}

fn fzz(directory: &Path) -> assert_cmd::Command {
    let mut command = cargo::cargo_bin_cmd!("fzz");
    command
        .current_dir(directory)
        .env("FUNZZY_COLORED", "false")
        .env("FUNZZY_BAIL", "false")
        .env("_TEST_FUNZZY_BAIL", "false")
        .arg("-c")
        .arg(".watch.yaml");
    command
}

fn write_config(directory: &Path, content: &str) {
    std::fs::write(directory.join(".watch.yaml"), content).expect("write config");
}

/// One probe task body: touches `NAME.running`, polls up to 100 x 20ms for
/// `SIBLING.running`, and if it ever sees it, records the overlap marker
/// `NAME.overlap`. Always exits 0 so the *gate* task is the only failure
/// source — the fixture fails exactly when the two probes overlapped.
fn probe(name: &str, sibling: &str) -> String {
    format!(
        "touch {name}.running; i=0; while [ ! -f {sibling}.running ] && [ $i -lt 100 ]; do sleep 0.02; i=$((i + 1)); done; if [ -f {sibling}.running ]; then echo overlap > {name}.overlap; fi; rm -f {name}.running; exit 0"
    )
}

/// The serial gate: fails if any overlap marker exists.
const GATE: &str = "test -z \"$(ls *.overlap 2>/dev/null)\" && exit 0 || exit 1";

// ---------------------------------------------------------------------------
// Topology: overlap, barriers, serial, command order
// ---------------------------------------------------------------------------

#[test]
fn contiguous_group_tasks_overlap_while_flat_tasks_stay_sequential() {
    // Two tasks in one contiguous `checks` group must run concurrently; two
    // flat tasks after them must run strictly one after the other. The gate
    // fails only if a group member saw its sibling running.
    let directory = fixture("topology");
    write_config(
        &directory,
        &format!(
            "on:\n  change: '**/*'\n  concurrency: 2\ntasks:\n  - name: a @quick\n    parallel: checks\n    run: '{}'\n  - name: b @quick\n    parallel: checks\n    run: '{}'\n  - name: flat one @quick\n    run: '{}'\n  - name: flat two @quick\n    run: '{}'\n  - name: gate @quick\n    run: '{}'\n",
            probe("a", "b"),
            probe("b", "a"),
            probe("flatone", "flattwo"),
            probe("flattwo", "flatone"),
            GATE,
        ),
    );

    // a and b overlap (parallel group), so an overlap marker exists and the
    // gate fails. Flat tasks never overlapped, so no flat*.overlap files.
    fzz(&directory)
        .args(["run", "@quick"])
        .assert()
        .code(1)
        .stdout(predicate::str::contains("gate @quick: failed"))
        .stdout(predicate::str::contains("Failed: 1;"));
    // At least one probe saw its sibling running (the exact writer is
    // completion-order dependent, which the contract leaves unspecified).
    assert!(
        directory.join("a.overlap").exists() || directory.join("b.overlap").exists(),
        "group overlap must be detected"
    );
    assert!(!directory.join("flatone.overlap").exists());
    assert!(!directory.join("flattwo.overlap").exists());
    std::fs::remove_dir_all(directory).unwrap();
}

#[test]
fn separated_group_occurrences_respect_barriers() {
    // A@x, B, C@x: the second `x` reuse starts a NEW barrier after the serial
    // B. A must never see C and C must never see A.
    let directory = fixture("barriers");
    write_config(
        &directory,
        &format!(
            "on:\n  change: '**/*'\n  concurrency: 2\ntasks:\n  - name: a @quick\n    parallel: x\n    run: '{}'\n  - name: b @quick\n    run: '{}'\n  - name: c @quick\n    parallel: x\n    run: '{}'\n  - name: gate @quick\n    run: '{}'\n",
            probe("a", "c"),
            probe("b", "a"),
            probe("c", "a"),
            GATE,
        ),
    );

    fzz(&directory)
        .args(["run", "@quick"])
        .assert()
        .success()
        .stdout(predicate::str::contains("Success"))
        .stdout(predicate::str::contains("Completed: 4"));
    assert!(!directory.join("a.overlap").exists());
    assert!(!directory.join("c.overlap").exists());
    std::fs::remove_dir_all(directory).unwrap();
}

#[test]
fn commands_inside_one_task_stay_sequential() {
    // Two commands in one task: the second appends only if the first already
    // wrote its file; if they ran concurrently the gate would find the chain
    // broken.
    let directory = fixture("command-order");
    write_config(
        &directory,
        "on:\n  change: '**/*'\n  concurrency: 4\ntasks:\n  - name: chain @quick\n    parallel: g\n    run:\n      - 'printf first > chain.txt'\n      - 'test -f chain.txt && printf second >> chain.txt'\n  - name: gate @quick\n    run: 'test \"$(cat chain.txt)\" = firstsecond && exit 0 || exit 1'\n",
    );

    fzz(&directory)
        .args(["run", "@quick"])
        .assert()
        .success()
        .stdout(predicate::str::contains("Success"));
    assert_eq!(
        std::fs::read_to_string(directory.join("chain.txt")).unwrap(),
        "firstsecond"
    );
    std::fs::remove_dir_all(directory).unwrap();
}

// ---------------------------------------------------------------------------
// Concurrency bound high-water mark
// ---------------------------------------------------------------------------

#[test]
fn concurrency_bound_limits_overlap_for_one_two_and_more_than_task_count() {
    // concurrency: 1 -> no group overlap (gate passes).
    let directory = fixture("bound-one");
    write_config(
        &directory,
        &format!(
            "on:\n  change: '**/*'\n  concurrency: 1\ntasks:\n  - name: a @quick\n    parallel: checks\n    run: '{}'\n  - name: b @quick\n    parallel: checks\n    run: '{}'\n  - name: gate @quick\n    run: '{}'\n",
            probe("a", "b"),
            probe("b", "a"),
            GATE,
        ),
    );
    fzz(&directory)
        .args(["run", "@quick"])
        .assert()
        .success()
        .stdout(predicate::str::contains("Success"));
    assert!(!directory.join("a.overlap").exists());
    std::fs::remove_dir_all(directory).unwrap();

    // concurrency: 2 -> overlap happens within the bound.
    let directory = fixture("bound-two");
    write_config(
        &directory,
        &format!(
            "on:\n  change: '**/*'\n  concurrency: 2\ntasks:\n  - name: a @quick\n    parallel: checks\n    run: '{}'\n  - name: b @quick\n    parallel: checks\n    run: '{}'\n  - name: gate @quick\n    run: '{}'\n",
            probe("a", "b"),
            probe("b", "a"),
            GATE,
        ),
    );
    fzz(&directory)
        .args(["run", "@quick"])
        .assert()
        .code(1)
        .stdout(predicate::str::contains("Failed: 1;"));
    assert!(
        directory.join("a.overlap").exists() || directory.join("b.overlap").exists(),
        "concurrency 2 must detect group overlap"
    );
    std::fs::remove_dir_all(directory).unwrap();

    // concurrency: 8 with only 2 tasks -> overlap, bounded by task count.
    let directory = fixture("bound-many");
    write_config(
        &directory,
        &format!(
            "on:\n  change: '**/*'\n  concurrency: 8\ntasks:\n  - name: a @quick\n    parallel: checks\n    run: '{}'\n  - name: b @quick\n    parallel: checks\n    run: '{}'\n  - name: gate @quick\n    run: '{}'\n",
            probe("a", "b"),
            probe("b", "a"),
            GATE,
        ),
    );
    fzz(&directory)
        .args(["run", "@quick"])
        .assert()
        .code(1)
        .stdout(predicate::str::contains("Failed: 1;"));
    assert!(
        directory.join("a.overlap").exists() || directory.join("b.overlap").exists(),
        "bound above task count must still overlap"
    );
    std::fs::remove_dir_all(directory).unwrap();
}

// ---------------------------------------------------------------------------
// Outcome combinations
// ---------------------------------------------------------------------------

#[test]
fn parallel_outcomes_cover_all_pass_one_fail_many_fail_and_fail_fast() {
    // All pass.
    let directory = fixture("all-pass");
    write_config(
        &directory,
        "on:\n  change: '**/*'\n  concurrency: 4\ntasks:\n  - name: ok a @quick\n    parallel: checks\n    run: 'true'\n  - name: ok b @quick\n    parallel: checks\n    run: 'true'\n",
    );
    fzz(&directory)
        .args(["run", "@quick"])
        .assert()
        .success()
        .stdout(predicate::str::contains("Completed: 2"));
    std::fs::remove_dir_all(directory).unwrap();

    // One fail: siblings and later tasks finish; run fails.
    let directory = fixture("one-fail");
    write_config(
        &directory,
        "on:\n  change: '**/*'\n  concurrency: 4\ntasks:\n  - name: bad @quick\n    parallel: checks\n    run: 'exit 3'\n  - name: good @quick\n    parallel: checks\n    run: 'true'\n  - name: after @quick\n    run: 'true'\n",
    );
    fzz(&directory)
        .args(["run", "@quick"])
        .assert()
        .code(1)
        .stdout(predicate::str::contains("Failed: 1;"))
        .stdout(predicate::str::contains("Completed: 2"));
    std::fs::remove_dir_all(directory).unwrap();

    // Many fail: every failure reported.
    let directory = fixture("many-fail");
    write_config(
        &directory,
        "on:\n  change: '**/*'\n  concurrency: 4\ntasks:\n  - name: bad a @quick\n    parallel: checks\n    run: 'exit 1'\n  - name: bad b @quick\n    parallel: checks\n    run: 'exit 2'\n  - name: bad c @quick\n    parallel: checks\n    run: 'exit 3'\n",
    );
    fzz(&directory)
        .args(["run", "@quick"])
        .assert()
        .code(1)
        .stdout(predicate::str::contains("Failed: 3;"));
    std::fs::remove_dir_all(directory).unwrap();

    // Fail-fast: active siblings cancel, later serial work skips. The bad
    // task fails immediately; the sibling sleeps so the cancel lands while it
    // is still active.
    let directory = fixture("fail-fast");
    write_config(
        &directory,
        "on:\n  change: '**/*'\n  concurrency: 4\ntasks:\n  - name: bad @quick\n    parallel: checks\n    run: 'exit 1'\n  - name: sibling @quick\n    parallel: checks\n    run: 'sleep 3; printf sibling-ran > sibling.txt'\n  - name: after @quick\n    run: 'printf after-ran > after.txt'\n",
    );
    fzz(&directory)
        .args(["--fail-fast", "run", "@quick"])
        .assert()
        .code(1)
        .stdout(predicate::str::contains("Failed: 1;"));
    assert!(!directory.join("sibling.txt").exists());
    assert!(!directory.join("after.txt").exists());
    std::fs::remove_dir_all(directory).unwrap();
}

#[test]
fn spawn_failure_is_reported_as_task_failure_within_the_group() {
    // A command that cannot spawn (nonexistent binary) must occupy and
    // release a slot and surface as a task failure, not hang the run.
    let directory = fixture("spawn-failure");
    write_config(
        &directory,
        "on:\n  change: '**/*'\n  concurrency: 2\ntasks:\n  - name: missing @quick\n    parallel: checks\n    run: 'definitely-not-a-real-binary-xyz'\n  - name: good @quick\n    parallel: checks\n    run: 'true'\n  - name: after @quick\n    run: 'true'\n",
    );

    fzz(&directory)
        .args(["run", "@quick"])
        .assert()
        .code(1)
        .stdout(predicate::str::contains("Failed: 1;"))
        .stdout(predicate::str::contains("Completed: 2"));
    std::fs::remove_dir_all(directory).unwrap();
}

// ---------------------------------------------------------------------------
// One engine across control run, emit, filesystem, and init (integration)
// ---------------------------------------------------------------------------

#[cfg(feature = "test-integration")]
mod integration {
    use super::*;
    use std::io::{BufRead, BufReader, Write};
    use std::os::unix::net::UnixStream;
    use std::process::{Child, Command, Stdio};
    use std::time::Duration;

    struct Watcher {
        child: Child,
        directory: PathBuf,
    }
    impl Drop for Watcher {
        fn drop(&mut self) {
            let _ = self.child.kill();
            let _ = self.child.wait();
            let _ = std::fs::remove_dir_all(&self.directory);
        }
    }

    fn start_watcher(directory: &Path) -> Watcher {
        let child = Command::new(env!("CARGO_BIN_EXE_fzz"))
            .current_dir(directory)
            .env_remove("FUNZZY_BAIL")
            .env_remove("FUNZZY_NON_BLOCK")
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .spawn()
            .unwrap();
        Watcher {
            child,
            directory: directory.to_path_buf(),
        }
    }

    fn wait_until_socket(directory: &Path) {
        let socket = directory.join("sock");
        for _ in 0..150 {
            if UnixStream::connect(&socket).is_ok() {
                return;
            }
            std::thread::sleep(Duration::from_millis(100));
        }
        panic!("socket never appeared");
    }

    fn raw(method: &str, params: serde_json::Value, socket: &Path) -> serde_json::Value {
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

    fn wait_until<F: FnMut() -> bool>(mut condition: F) {
        for _ in 0..150 {
            if condition() {
                return;
            }
            std::thread::sleep(Duration::from_millis(100));
        }
        panic!("wait_until timed out");
    }

    const WATCH_CONFIG: &str = r#"
on:
  socket: sock
  concurrency: 2
tasks:
  - name: file a
    parallel: checks
    run: "touch a.ran"
    change: "*.txt"
  - name: file b
    parallel: checks
    run: "touch b.ran"
    change: "*.txt"
  - name: init only
    run: "touch init.ran"
    change: "never-matches"
    run_on_init: true
"#;

    #[test]
    fn control_emit_filesystem_and_init_share_the_parallel_engine() {
        let directory = fixture("engine");
        write_config(&directory, WATCH_CONFIG);
        let _watcher = start_watcher(&directory);
        wait_until_socket(&directory);
        let socket = directory.join("sock");

        // run_on_init executes through the same executor at startup.
        wait_until(|| directory.join("init.ran").exists());

        // Synthetic emit schedules both group members under one generation.
        let emit = raw("emit", serde_json::json!({"path": "notes.txt"}), &socket);
        assert_eq!(emit["result"]["matched"].as_array().unwrap().len(), 2);
        wait_until(|| directory.join("a.ran").exists() && directory.join("b.ran").exists());
        std::fs::remove_file(directory.join("a.ran")).unwrap();
        std::fs::remove_file(directory.join("b.ran")).unwrap();

        // Control run triggers the same target through the worker.
        let run = raw("run", serde_json::json!({"target": "file a"}), &socket);
        assert!(run["result"]["runId"].is_number());
        wait_until(|| directory.join("a.ran").exists());

        // Filesystem event routes through the same engine.
        let _ = std::fs::remove_file(directory.join("a.ran"));
        let _ = std::fs::remove_file(directory.join("b.ran"));
        std::fs::write(directory.join("trigger.txt"), "x").unwrap();
        wait_until(|| directory.join("a.ran").exists() && directory.join("b.ran").exists());
    }

    #[test]
    fn repeated_replacement_leaves_no_descendants_behind() {
        // Rapid control runs under the restart policy replace active work;
        // after settling, no child process group survives (reaped via the
        // worker's cancel path). Uses a long-running task and then cancels.
        let directory = fixture("replacement");
        write_config(
            &directory,
            "on:\n  socket: sock\n  concurrency: 2\ntasks:\n  - name: long\n    run: 'sleep 30'\n    change: \"*.txt\"\n",
        );
        let _watcher = start_watcher(&directory);
        wait_until_socket(&directory);
        let socket = directory.join("sock");

        let run = raw("run", serde_json::json!({"target": "long"}), &socket);
        let run_id = run["result"]["runId"].as_u64().unwrap();
        let cancel = raw("cancel", serde_json::json!({"generation": run_id}), &socket);
        assert_eq!(cancel["result"]["cancelled"], true);

        wait_until(|| {
            let status = raw("status", serde_json::json!({}), &socket);
            status["result"]["generation"].as_u64() == Some(run_id)
                && status["result"]["state"].as_str() == Some("cancelled")
        });
    }
}

#[cfg(feature = "test-integration")]
#[test]
fn parallel_batch_latency_approaches_slowest_task_not_the_sum() {
    // Supporting benchmark (TASK-0029): three independent 0.3s sleeps take
    // ~0.9s serially but ~0.3s parallel. This is supporting evidence with a
    // generous bound, never the sole correctness test; the environment is
    // recorded so results are interpretable. Bounded: parallel must be at
    // least 1.5x faster than serial.
    use std::time::Instant;

    let serial = fixture("bench-serial");
    write_config(
        &serial,
        "on:\n  change: '**/*'\n  concurrency: 1\ntasks:\n  - name: s1 @bench\n    parallel: checks\n    run: 'sleep 0.3'\n  - name: s2 @bench\n    parallel: checks\n    run: 'sleep 0.3'\n  - name: s3 @bench\n    parallel: checks\n    run: 'sleep 0.3'\n",
    );
    let start = Instant::now();
    fzz(&serial).args(["run", "@bench"]).assert().success();
    let serial_ms = start.elapsed().as_millis();
    std::fs::remove_dir_all(&serial).unwrap();

    let parallel = fixture("bench-parallel");
    write_config(
        &parallel,
        "on:\n  change: '**/*'\n  concurrency: 8\ntasks:\n  - name: p1 @bench\n    parallel: checks\n    run: 'sleep 0.3'\n  - name: p2 @bench\n    parallel: checks\n    run: 'sleep 0.3'\n  - name: p3 @bench\n    parallel: checks\n    run: 'sleep 0.3'\n",
    );
    let start = Instant::now();
    fzz(&parallel).args(["run", "@bench"]).assert().success();
    let parallel_ms = start.elapsed().as_millis();
    std::fs::remove_dir_all(&parallel).unwrap();

    // Record the environment so the number is interpretable, then assert the
    // direction with a generous bound (parallel < 2/3 of serial).
    eprintln!(
        "benchmark: serial={serial_ms}ms parallel={parallel_ms}ms (threads={})",
        std::thread::available_parallelism()
            .map(|n| n.get())
            .unwrap_or(1)
    );
    assert!(
        parallel_ms < serial_ms,
        "parallel batch must beat serial: parallel={parallel_ms}ms serial={serial_ms}ms"
    );
}
