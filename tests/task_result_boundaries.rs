//! Narrow source guard for the task-result ownership boundary.
//!
//! Executor still uses stdout for diagnostics, so the forbidden shape is the
//! direct cycle: executor -> stdout plus stdout -> executor result types.
//! The contract owner breaks that cycle without changing diagnostics.

use std::fs;
use std::path::Path;

#[test]
fn task_result_owner_prevents_the_executor_stdout_cycle() {
    let root = Path::new(env!("CARGO_MANIFEST_DIR"));
    let executor = fs::read_to_string(root.join("src/executor.rs")).unwrap();
    let stdout = fs::read_to_string(root.join("src/stdout.rs")).unwrap();

    assert!(executor_imports_stdout(&executor));
    assert!(stdout_imports_task_result_owner(&stdout));
    assert!(!stdout_imports_executor_results(&stdout));
    assert_no_direct_cycle(&executor, &stdout);
}

#[test]
fn task_result_guard_rejects_either_cycle_shape() {
    let executor = "use crate::stdout;";
    let stdout = "use crate::executor::TaskSnapshot;";
    assert!(cycle_error(executor, stdout).is_some());

    let executor = "use crate::diagnostics;";
    let stdout = "use crate::executor::TaskState;";
    assert!(cycle_error(executor, stdout).is_none());
}

fn assert_no_direct_cycle(executor: &str, stdout: &str) {
    if let Some(error) = cycle_error(executor, stdout) {
        panic!("{error}");
    }
}

fn cycle_error(executor: &str, stdout: &str) -> Option<&'static str> {
    if executor_imports_stdout(executor) && stdout_imports_executor_results(stdout) {
        Some("executor/stdout result contract cycle detected")
    } else {
        None
    }
}

fn executor_imports_stdout(source: &str) -> bool {
    source
        .lines()
        .any(|line| line.trim() == "use crate::stdout;")
}

fn stdout_imports_executor_results(source: &str) -> bool {
    source.lines().any(|line| {
        let line = line.trim();
        line.starts_with("use crate::executor")
            || line.contains("crate::executor::TaskSnapshot")
            || line.contains("crate::executor::TaskState")
    })
}

fn stdout_imports_task_result_owner(source: &str) -> bool {
    source
        .lines()
        .any(|line| line.trim() == "use crate::task_result::{TaskSnapshot, TaskState};")
}
