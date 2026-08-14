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
        .stdout(predicate::str::contains("Available tasks"));

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
