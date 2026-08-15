//! Black-box contract for finite local configured workflow execution (TASK-0038).

use assert_cmd::cargo;
use predicates::prelude::*;
use std::path::{Path, PathBuf};

fn fixture(name: &str) -> PathBuf {
    let directory =
        std::env::temp_dir().join(format!("funzzy-run-once-{}-{}", std::process::id(), name));
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

#[test]
fn exact_target_runs_once_with_configured_context_and_without_watcher_ipc() {
    let directory = fixture("exact");
    std::fs::create_dir_all(directory.join("service path")).unwrap();
    write_config(
        &directory,
        "on:\n  change: '**/*'\n  socket: local.sock\ntasks:\n  - name: build\n    cwd: service path\n    env: { EXPECTED: exact }\n    run: 'test \"$EXPECTED\" = exact && printf exact > exact.txt'\n  - name: build docs\n    run: 'printf docs > docs.txt'\n",
    );

    fzz(&directory)
        .args(["run", "build"])
        .assert()
        .success()
        .stdout(predicate::str::contains("Success"))
        .stdout(predicate::str::contains("Watching...").not());

    assert_eq!(
        std::fs::read_to_string(directory.join("service path/exact.txt")).unwrap(),
        "exact"
    );
    assert!(!directory.join("docs.txt").exists());
    assert!(!directory.join("local.sock").exists());
    std::fs::remove_dir_all(directory).unwrap();
}

#[test]
fn tag_selection_runs_all_matching_parallel_tasks() {
    let directory = fixture("tag-parallel");
    write_config(
        &directory,
        "on:\n  change: '**/*'\n  concurrency: 2\ntasks:\n  - name: lint @quick\n    parallel: checks\n    run: 'touch lint.ready; i=0; while [ $i -lt 100 ]; do test -f test.ready && exit 0; i=$((i + 1)); sleep 0.02; done; exit 1'\n  - name: test @quick\n    parallel: checks\n    run: 'touch test.ready; i=0; while [ $i -lt 100 ]; do test -f lint.ready && exit 0; i=$((i + 1)); sleep 0.02; done; exit 1'\n",
    );

    fzz(&directory)
        .args(["run", "@quick"])
        .assert()
        .success()
        .stdout(predicate::str::contains("Completed: 2"));

    assert!(directory.join("lint.ready").exists());
    assert!(directory.join("test.ready").exists());
    std::fs::remove_dir_all(directory).unwrap();
}

#[test]
fn parallel_live_output_is_attributed_and_summary_names_group_and_tasks() {
    // TASK-0028: live lines from parallel-group tasks carry the `[task]`
    // prefix (identity under interleaving), and the final summary lists every
    // task with its group. The output is deterministic in content, not in
    // completion order, so assertions are task-keyed contains-checks.
    let directory = fixture("attributed-output");
    write_config(
        &directory,
        "on:\n  change: '**/*'\n  concurrency: 2\ntasks:\n  - name: lint @quick\n    parallel: checks\n    run: 'echo lint-output; echo lint-second'\n  - name: test @quick\n    parallel: checks\n    run: 'echo test-output'\n",
    );

    fzz(&directory)
        .args(["run", "@quick"])
        .assert()
        .success()
        // Live lines are attributed to the emitting task.
        .stdout(predicate::str::contains("[lint @quick] lint-output"))
        .stdout(predicate::str::contains("[lint @quick] lint-second"))
        .stdout(predicate::str::contains("[test @quick] test-output"))
        // Final summary identifies group and every task.
        .stdout(predicate::str::contains("- [checks#1] lint @quick: passed"))
        .stdout(predicate::str::contains("- [checks#1] test @quick: passed"))
        .stdout(predicate::str::contains("Completed: 2"));

    std::fs::remove_dir_all(directory).unwrap();
}

#[test]
fn sequential_override_forces_effective_concurrency_one() {
    // SEQUENTIAL-OVERRIDE-CONTRACT §2: the handshake tasks only pass when
    // both run concurrently (concurrency 2). Under `--sequential` the first
    // task never sees the second's ready file and must fail; selection,
    // plan barriers, and commands stay unchanged.
    let directory = fixture("sequential");
    write_config(
        &directory,
        "on:\n  change: '**/*'\n  concurrency: 2\ntasks:\n  - name: lint @quick\n    parallel: checks\n    run: 'touch lint.ready; i=0; while [ $i -lt 100 ]; do test -f test.ready && exit 0; i=$((i + 1)); sleep 0.02; done; exit 1'\n  - name: test @quick\n    parallel: checks\n    run: 'touch test.ready; i=0; while [ $i -lt 100 ]; do test -f lint.ready && exit 0; i=$((i + 1)); sleep 0.02; done; exit 1'\n",
    );

    // Baseline: configured concurrency 2 lets both tasks pass concurrently.
    fzz(&directory)
        .args(["run", "@quick"])
        .assert()
        .success()
        .stdout(predicate::str::contains("Completed: 2"));
    assert!(directory.join("lint.ready").exists());
    assert!(directory.join("test.ready").exists());
    std::fs::remove_file(directory.join("lint.ready")).unwrap();
    std::fs::remove_file(directory.join("test.ready")).unwrap();

    // Override: `--sequential` must make the first task wait in vain.
    fzz(&directory)
        .args(["run", "@quick", "--sequential"])
        .assert()
        .code(1)
        .stdout(predicate::str::contains("Completed: 1"))
        .stdout(predicate::str::contains("has failed"));
    assert!(directory.join("lint.ready").exists());
    std::fs::remove_dir_all(directory).unwrap();
}

#[test]
fn missing_and_ambiguous_targets_are_actionable() {
    let directory = fixture("selection-errors");
    write_config(
        &directory,
        "- name: lint api\n  run: 'true'\n  change: '**/*'\n- name: lint web\n  run: 'true'\n  change: '**/*'\n",
    );

    fzz(&directory)
        .args(["run", "missing"])
        .assert()
        .code(1)
        .stdout(predicate::str::contains("No target found for 'missing'"))
        .stdout(predicate::str::contains("Available jobs"));

    fzz(&directory)
        .args(["run", "lint"])
        .assert()
        .code(1)
        .stdout(predicate::str::contains("Target 'lint' is ambiguous"))
        .stdout(predicate::str::contains("lint api"))
        .stdout(predicate::str::contains("lint web"));

    std::fs::remove_dir_all(directory).unwrap();
}

#[test]
fn failure_status_combines_tasks_and_fail_fast_stops_remaining_work() {
    let directory = fixture("failure");
    write_config(
        &directory,
        "- name: first @failure\n  run: 'exit 7'\n  change: '**/*'\n- name: after @failure\n  run: 'printf continued > after.txt'\n  change: '**/*'\n",
    );

    fzz(&directory)
        .args(["run", "@failure"])
        .assert()
        .code(1)
        .stdout(predicate::str::contains("Failure"));
    assert_eq!(
        std::fs::read_to_string(directory.join("after.txt")).unwrap(),
        "continued"
    );

    std::fs::remove_file(directory.join("after.txt")).unwrap();
    fzz(&directory)
        .args(["--fail-fast", "run", "@failure"])
        .assert()
        .code(1)
        .stdout(predicate::str::contains("Failure"));
    assert!(!directory.join("after.txt").exists());
    std::fs::remove_dir_all(directory).unwrap();
}

#[test]
fn run_rejects_path_input_instead_of_silently_filtering() {
    let directory = fixture("path-rejected");
    write_config(
        &directory,
        "- name: build\n  run: 'true'\n  change: '**/*'\n",
    );

    fzz(&directory)
        .args(["run", "build", "src/lib.rs"])
        .assert()
        .code(2)
        .stderr(predicate::str::contains("unexpected argument"));

    std::fs::remove_dir_all(directory).unwrap();
}

#[cfg(feature = "test-integration")]
#[test]
fn ctrl_c_stops_owned_task_group_and_exits_130() {
    use std::process::Stdio;
    use std::time::{Duration, Instant};

    let directory = fixture("ctrl-c");
    write_config(
        &directory,
        "- name: service\n  run: \"bash -c 'sleep 30 & echo $! > child.pid; wait'\"\n  change: '**/*'\n",
    );

    let mut child = std::process::Command::new(env!("CARGO_BIN_EXE_fzz"))
        .current_dir(&directory)
        .env("FUNZZY_COLORED", "false")
        .args(["-c", ".watch.yaml", "run", "service"])
        .stdout(Stdio::null())
        .spawn()
        .expect("spawn run");

    let pid_file = directory.join("child.pid");
    let ready_deadline = Instant::now() + Duration::from_secs(5);
    while !pid_file.exists() {
        assert!(Instant::now() < ready_deadline, "task did not start");
        std::thread::sleep(Duration::from_millis(20));
    }
    let task_pid = std::fs::read_to_string(&pid_file).unwrap();

    let signal = std::process::Command::new("kill")
        .args(["-INT", &child.id().to_string()])
        .status()
        .expect("send Ctrl-C");
    assert!(signal.success());

    let exit_deadline = Instant::now() + Duration::from_secs(10);
    let status = loop {
        if let Some(status) = child.try_wait().expect("poll run") {
            break status;
        }
        assert!(Instant::now() < exit_deadline, "run did not exit");
        std::thread::sleep(Duration::from_millis(20));
    };
    assert_eq!(status.code(), Some(130));

    let alive = std::process::Command::new("kill")
        .args(["-0", task_pid.trim()])
        .output()
        .map(|output| output.status.success())
        .unwrap_or(false);
    assert!(!alive, "Ctrl-C orphaned child {}", task_pid.trim());
    std::fs::remove_dir_all(directory).unwrap();
}

#[test]
fn parallel_binary_output_is_lossy_attributed_and_never_drops_stream() {
    // TASK-0028: a parallel-group task emitting invalid-UTF-8 bytes must not
    // lose the rest of its stream (the old read_line loop broke on the first
    // invalid byte); every line still carries the task attribution.
    let directory = fixture("binary-output");
    write_config(
        &directory,
        "on:\n  change: '**/*'\n  concurrency: 2\ntasks:\n  - name: binary @quick\n    parallel: checks\n    run: 'printf \"\\377\\376first\\nsecond\\n\"'\n  - name: plain @quick\n    parallel: checks\n    run: 'echo plain-output'\n",
    );

    let output = fzz(&directory)
        .args(["run", "@quick"])
        .output()
        .expect("run");
    assert!(output.status.success());
    let stdout = String::from_utf8_lossy(&output.stdout);
    // Both lines from the binary task survive, attributed to the task. The
    // invalid bytes render lossily (replacement chars) before "first".
    assert!(
        stdout.contains("[binary @quick]") && stdout.contains("first"),
        "first line must survive with attribution: {stdout}"
    );
    assert!(
        stdout.contains("[binary @quick] second"),
        "second line must survive after invalid bytes: {stdout}"
    );
    assert!(
        stdout.contains("[plain @quick] plain-output"),
        "sibling task output: {stdout}"
    );
    assert!(
        stdout.contains("- [checks#1] binary @quick: passed")
            && stdout.contains("- [checks#1] plain @quick: passed"),
        "summary must list every task with its group: {stdout}"
    );
    std::fs::remove_dir_all(directory).unwrap();
}

#[test]
fn parallel_partial_final_line_without_newline_is_emitted() {
    // TASK-0028: a child that exits without a trailing newline must still
    // have its final partial line forwarded and attributed.
    let directory = fixture("partial-line");
    write_config(
        &directory,
        "on:\n  change: '**/*'\n  concurrency: 2\ntasks:\n  - name: partial @quick\n    parallel: checks\n    run: 'printf \"complete\\nno-newline\"'\n  - name: plain @quick\n    parallel: checks\n    run: 'echo plain-output'\n",
    );

    let output = fzz(&directory)
        .args(["run", "@quick"])
        .output()
        .expect("run");
    assert!(output.status.success());
    let stdout = String::from_utf8_lossy(&output.stdout);
    assert!(
        stdout.contains("[partial @quick] complete"),
        "complete line: {stdout}"
    );
    assert!(
        stdout.contains("[partial @quick] no-newline"),
        "partial final line must still be emitted: {stdout}"
    );
    assert!(
        stdout.contains("- [checks#1] partial @quick: passed"),
        "summary: {stdout}"
    );
    std::fs::remove_dir_all(directory).unwrap();
}

#[test]
fn parallel_sensitive_workflow_fails_when_overlapping_and_passes_sequential() {
    // TASK-0074: deterministic parallel-vs-sequential proof. Two probe tasks
    // in one parallel group each touch a `running` marker and poll for their
    // sibling's marker; seeing it proves overlap and writes an `overlap`
    // file. A serial gate task fails when that file exists. Under configured
    // concurrency 2 the probes overlap and the gate fails; under the explicit
    // `--sequential` override nothing overlaps and the gate passes. No wall
    // clocks or probabilities: the outcome is a deterministic function of
    // whether the two probes ran concurrently.
    let config = "on:\n  change: '**/*'\n  concurrency: 2\ntasks:\n  - name: probe a @quick\n    parallel: checks\n    run: 'touch a.running; i=0; while [ ! -f b.running ] && [ $i -lt 100 ]; do sleep 0.02; i=$((i + 1)); done; if [ -f b.running ]; then echo overlap > overlap; fi; rm -f a.running; exit 0'\n  - name: probe b @quick\n    parallel: checks\n    run: 'touch b.running; i=0; while [ ! -f a.running ] && [ $i -lt 100 ]; do sleep 0.02; i=$((i + 1)); done; if [ -f a.running ]; then echo overlap > overlap; fi; rm -f b.running; exit 0'\n  - name: gate @quick\n    run: 'test ! -f overlap && exit 0 || exit 1'\n";

    // Parallel baseline: the two probes overlap, the gate fails, run fails.
    let directory = fixture("parallel-sensitive");
    write_config(&directory, config);
    fzz(&directory)
        .args(["run", "@quick"])
        .assert()
        .code(1)
        .stdout(predicate::str::contains("gate @quick: failed"))
        .stdout(predicate::str::contains("Failed: 1;"));
    assert!(directory.join("overlap").exists());
    std::fs::remove_file(directory.join("overlap")).unwrap();
    std::fs::remove_dir_all(directory).unwrap();

    // Sequential override: same target, same commands, only effective
    // concurrency differs; the gate passes and the run succeeds.
    let directory = fixture("parallel-sensitive-seq");
    write_config(&directory, config);
    fzz(&directory)
        .args(["run", "@quick", "--sequential"])
        .assert()
        .success()
        .stdout(predicate::str::contains("gate @quick: passed"))
        .stdout(predicate::str::contains("Success"));
    assert!(!directory.join("overlap").exists());
    std::fs::remove_dir_all(directory).unwrap();
}

#[test]
fn jobs_config_runs_identically_to_tasks() {
    // TASK-0076: the preferred `jobs:` vocabulary flows through the same
    // matching, execution, and combined-result paths as `tasks:` — identical
    // semantics, identical outcome.
    let directory = fixture("jobs-config");
    write_config(
        &directory,
        "on:\n  change: '**/*'\njobs:\n  - name: lint @quick\n    run: 'echo lint-done > lint.txt'\n  - name: test @quick\n    run: 'echo test-done > test.txt'\n",
    );

    fzz(&directory)
        .args(["run", "@quick"])
        .assert()
        .success()
        .stdout(predicate::str::contains("Completed: 2"));
    assert_eq!(
        std::fs::read_to_string(directory.join("lint.txt")).unwrap(),
        "lint-done\n"
    );
    assert_eq!(
        std::fs::read_to_string(directory.join("test.txt")).unwrap(),
        "test-done\n"
    );
    std::fs::remove_dir_all(directory).unwrap();
}

#[test]
fn jobs_parallel_group_keeps_barriers_and_execution_semantics() {
    // TASK-0076: `parallel:` groups inside `jobs:` behave exactly as with
    // `tasks:` — declaration order and contiguous barriers are preserved.
    let directory = fixture("jobs-parallel");
    write_config(
        &directory,
        "on:\n  change: '**/*'\n  concurrency: 2\njobs:\n  - name: a @quick\n    parallel: checks\n    run: 'touch a.ready; i=0; while [ $i -lt 100 ]; do test -f b.ready && exit 0; i=$((i + 1)); sleep 0.02; done; exit 1'\n  - name: b @quick\n    parallel: checks\n    run: 'touch b.ready; i=0; while [ $i -lt 100 ]; do test -f a.ready && exit 0; i=$((i + 1)); sleep 0.02; done; exit 1'\n",
    );

    fzz(&directory)
        .args(["run", "@quick"])
        .assert()
        .success()
        .stdout(predicate::str::contains("Completed: 2"));
    assert!(directory.join("a.ready").exists());
    assert!(directory.join("b.ready").exists());
    std::fs::remove_dir_all(directory).unwrap();
}

#[test]
fn run_events_flag_writes_ndjson_stream() {
    // TASK-0039: `--events FILE` appends NDJSON run events; the stream must
    // contain started/task_terminal/finished records with schema version,
    // generation identity, and the final order-independent outcome.
    let directory = fixture("events");
    write_config(
        &directory,
        "on:\n  change: '**/*'\njobs:\n  - name: ok @quick\n    run: 'true'\n  - name: bad @quick\n    run: 'exit 1'\n",
    );
    let events_path = directory.join("run-events.ndjson");

    fzz(&directory)
        .args(["run", "@quick", "--events", events_path.to_str().unwrap()])
        .assert()
        .code(1);

    let content = std::fs::read_to_string(&events_path).expect("events file");
    let records: Vec<serde_json::Value> = content
        .lines()
        .map(|line| serde_json::from_str(line).expect("valid ndjson"))
        .collect();
    assert!(!records.is_empty(), "stream must not be empty");
    for record in &records {
        assert_eq!(record["schemaVersion"], 1, "schema on every record");
        assert!(record["tsMs"].is_number());
    }
    assert!(
        records.iter().any(|r| r["event"] == "started"),
        "started record: {content}"
    );
    assert!(
        records
            .iter()
            .any(|r| r["event"] == "task_terminal" && r["state"] == "failed"),
        "failed task_terminal: {content}"
    );
    let finished = records
        .iter()
        .find(|r| r["event"] == "finished")
        .expect("finished record");
    assert!(
        finished["failures"]
            .as_array()
            .map(|f| !f.is_empty())
            .unwrap_or(false),
        "finished must carry order-independent failures: {finished}"
    );
    // Every task record has stable identity.
    assert!(
        records
            .iter()
            .all(|r| r["runId"].is_number() || r["event"] == "tick"),
        "run identity on records: {content}"
    );
    std::fs::remove_dir_all(directory).unwrap();
}

#[test]
fn success_and_failure_hooks_run_once_per_generation() {
    // TASK-0040: on.success runs once on pass, on.failure runs once on fail;
    // hook failure never changes the combined outcome.
    let directory = fixture("hooks");
    write_config(
        &directory,
        "on:\n  change: '**/*'\n  success: 'echo ok > hook-success.txt'\n  failure: 'echo bad > hook-failure.txt'\njobs:\n  - name: good @quick\n    run: 'true'\n  - name: bad @quick\n    run: 'exit 1'\n",
    );

    // Failing run: only the failure hook fires; exit stays 1.
    fzz(&directory).args(["run", "bad"]).assert().code(1);
    assert_eq!(
        std::fs::read_to_string(directory.join("hook-failure.txt")).unwrap(),
        "bad\n"
    );
    assert!(!directory.join("hook-success.txt").exists());

    // Passing run: only the success hook fires; exit 0.
    fzz(&directory).args(["run", "good"]).assert().code(0);
    assert_eq!(
        std::fs::read_to_string(directory.join("hook-success.txt")).unwrap(),
        "ok\n"
    );
    std::fs::remove_dir_all(directory).unwrap();
}
