//! Executor contract tests (TASK-0026).
//!
//! Acceptance: the same plan produces equivalent outcomes in wait and
//! restart modes at concurrency one. These tests guard the executor
//! unification: whichever busy-run policy drives the shared command loop,
//! the observed task outcome (failure attribution) must match.

use std::io::prelude::*;

#[path = "./common/lib.rs"]
mod setup;

/// A minimal failing plan: one task running `false` on init so both modes
/// execute it immediately and report the same child failure.
const FAILING_CONFIG: &str = "- name: failing
  run: 'false'
  change: 'trigger.txt'
  run_on_init: true
";

/// Wait mode (blocking, the default): the shared executor runs the command
/// synchronously and reports the child failure.
#[test]
fn failing_plan_reports_same_outcome_in_wait_mode() {
    setup::with_output(
        "executor_contract_wait_mode.log",
        |fzz_cmd, mut output_log, fixture| {
            std::fs::write(fixture.join(".watch.yaml"), FAILING_CONFIG)
                .expect("write fixture config");

            let mut child = fzz_cmd.spawn().expect("failed to spawn fzz");
            defer!({
                let _ = child.kill();
            });

            let mut output = String::new();
            wait_until!(
                {
                    output_log.read_to_string(&mut output).expect("read output");
                    output.contains("Funzzy results")
                },
                "wait mode never reported results: {}",
                output
            );

            assert!(
                output.contains("Command false has failed with exit status: 1"),
                "wait mode outcome mismatch:\n{}",
                output
            );
        },
    );
}

/// Restart mode (`--restart`): the worker drives the same command loop and
/// must report the identical child failure for the same plan.
#[test]
fn failing_plan_reports_same_outcome_in_restart_mode() {
    setup::with_output(
        "executor_contract_restart_mode.log",
        |fzz_cmd, mut output_log, fixture| {
            std::fs::write(fixture.join(".watch.yaml"), FAILING_CONFIG)
                .expect("write fixture config");

            let mut child = fzz_cmd
                .arg("--restart")
                .spawn()
                .expect("failed to spawn fzz");
            defer!({
                let _ = child.kill();
            });

            let mut output = String::new();
            wait_until!(
                {
                    output_log.read_to_string(&mut output).expect("read output");
                    output.contains("Funzzy results")
                },
                "restart mode never reported results: {}",
                output
            );

            // Same plan, same outcome as wait mode — the equivalence contract.
            assert!(
                output.contains("Command false has failed with exit status: 1"),
                "restart mode outcome mismatch:\n{}",
                output
            );
        },
    );
}
