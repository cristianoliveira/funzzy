//! TASK-0133: `fzz check` warning for init-only services (SERVICE-LIFECYCLE-CONTRACT §6).
//!
//! Black-box proof against the installed binary: the warning is actionable,
//! side-effect-free, and fires only for `service: true` + `run_on_init: true`
//! with empty *effective* change patterns. Every other shape stays silent and
//! `check` keeps exiting 0 — the config is legal-but-surprising, not invalid.

use assert_cmd::cargo;
use predicates::prelude::*;
use std::fs;
use std::path::{Path, PathBuf};
use std::time::{SystemTime, UNIX_EPOCH};

fn fixture(label: &str) -> PathBuf {
    let unique = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .expect("clock after epoch")
        .as_nanos();
    let directory = std::env::temp_dir().join(format!(
        "funzzy-service-check-{}-{}-{}",
        std::process::id(),
        unique,
        label
    ));
    let _ = fs::remove_dir_all(&directory);
    fs::create_dir_all(&directory).expect("create fixture");
    directory
}

fn fzz(directory: &Path) -> assert_cmd::Command {
    let mut command = cargo::cargo_bin_cmd!("fzz");
    command
        .current_dir(directory)
        .env("FUNZZY_COLORED", "false")
        .env("_TEST_FUNZZY_COLORED", "false")
        .arg("-c")
        .arg(".watch.yaml");
    command
}

fn write_config(directory: &Path, content: &str) {
    fs::write(directory.join(".watch.yaml"), content).expect("write config");
}

#[test]
fn init_only_service_warns_and_stays_valid() {
    let directory = fixture("init-only");
    write_config(
        &directory,
        "jobs:\n  - name: mirror\n    run: 'while true; do sleep 1; done'\n    service: true\n    run_on_init: true\n",
    );

    fzz(&directory)
        .arg("check")
        .assert()
        .code(0)
        .stdout(predicate::str::contains("Funzzy warning:"))
        .stdout(predicate::str::contains("init-only service"))
        .stdout(predicate::str::contains("mirror"))
        // Actionable: both contract-recommended fixes are named.
        .stdout(predicate::str::contains("change:"))
        .stdout(predicate::str::contains("dedicated config"))
        .stdout(predicate::str::contains("config valid: 1 job(s)"));

    fs::remove_dir_all(&directory).unwrap();
}

#[test]
fn service_with_job_level_change_stays_silent() {
    let directory = fixture("job-change");
    write_config(
        &directory,
        "jobs:\n  - name: mirror\n    run: 'sleep 10'\n    service: true\n    run_on_init: true\n    change: 'mirror/**'\n",
    );

    fzz(&directory)
        .arg("check")
        .assert()
        .code(0)
        .stdout(predicate::str::contains("config valid: 1 job(s)"))
        .stdout(predicate::str::contains("init-only service(s)").not());

    fs::remove_dir_all(&directory).unwrap();
}

#[test]
fn service_inheriting_root_change_stays_silent() {
    let directory = fixture("root-change");
    write_config(
        &directory,
        "on:\n  change: 'src/**'\njobs:\n  - name: mirror\n    run: 'sleep 10'\n    service: true\n    run_on_init: true\n",
    );

    fzz(&directory)
        .arg("check")
        .assert()
        .code(0)
        .stdout(predicate::str::contains("init-only service(s)").not());

    fs::remove_dir_all(&directory).unwrap();
}

#[test]
fn run_on_init_job_without_service_stays_silent() {
    let directory = fixture("plain-init");
    write_config(
        &directory,
        "jobs:\n  - name: setup\n    run: 'echo ready'\n    run_on_init: true\n",
    );

    fzz(&directory)
        .arg("check")
        .assert()
        .code(0)
        .stdout(predicate::str::contains("init-only service(s)").not());

    fs::remove_dir_all(&directory).unwrap();
}

#[test]
fn service_without_run_on_init_stays_silent() {
    let directory = fixture("no-init");
    write_config(
        &directory,
        "jobs:\n  - name: ondemand\n    run: 'sleep 10'\n    service: true\n    change: 'src/**'\n  - name: other\n    run: 'echo hi'\n    change: 'src/**'\n",
    );

    fzz(&directory)
        .arg("check")
        .assert()
        .code(0)
        .stdout(predicate::str::contains("init-only service(s)").not());

    fs::remove_dir_all(&directory).unwrap();
}
