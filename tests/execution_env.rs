//! Black-box coverage for project-level `execution.env` inheritance.

use assert_cmd::cargo;
use predicates::prelude::PredicateBooleanExt;
use std::fs;
use std::path::{Path, PathBuf};
use std::time::{SystemTime, UNIX_EPOCH};

fn fixture(label: &str) -> PathBuf {
    let unique = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .expect("clock after epoch")
        .as_nanos();
    let directory = std::env::temp_dir().join(format!(
        "funzzy-execution-env-{}-{unique}-{label}",
        std::process::id()
    ));
    fs::create_dir_all(&directory).expect("create fixture");
    directory
}

fn fzz(directory: &Path) -> assert_cmd::Command {
    let mut command = cargo::cargo_bin_cmd!("fzz");
    command
        .current_dir(directory)
        .env("FUNZZY_COLORED", "false")
        .env("FUNZZY_BAIL", "false")
        .arg("-c")
        .arg(".watch.yaml");
    command
}

#[test]
fn execution_defaults_reach_jobs_and_hooks_with_job_and_reserved_precedence() {
    let directory = fixture("run");
    fs::write(
        directory.join(".watch.yaml"),
        r#"on:
  change: '**/*'
execution:
  env:
    PROJECT_DEFAULT: from-execution
    OVERRIDE: from-execution
    EMPTY: ''
    FUNZZY_GENERATION_ID: configured-id
    FUNZZY_GENERATION_OUTCOME: configured-outcome
hooks:
  success: 'printf "%s|%s|%s" "$PROJECT_DEFAULT" "$FUNZZY_GENERATION_ID" "$FUNZZY_GENERATION_OUTCOME" > success.txt'
  failure: 'printf "%s|%s|%s" "$PROJECT_DEFAULT" "$FUNZZY_GENERATION_ID" "$FUNZZY_GENERATION_OUTCOME" > failure.txt'
jobs:
  - name: success
    run: 'test "$PROJECT_DEFAULT" = from-execution && test "$OVERRIDE" = from-job && test "$EMPTY" = "" && test "$PARENT_VALUE" = inherited && printf ok > job.txt'
    env:
      OVERRIDE: from-job
  - name: failure
    run: 'test "$PROJECT_DEFAULT" = from-execution && test "$OVERRIDE" = from-execution && exit 1'
"#,
    )
    .expect("write config");

    fzz(&directory)
        .env("PARENT_VALUE", "inherited")
        .args(["run", "success"])
        .assert()
        .success();
    assert_eq!(fs::read_to_string(directory.join("job.txt")).unwrap(), "ok");
    let success_hook = fs::read_to_string(directory.join("success.txt")).unwrap();
    let mut fields = success_hook.split('|');
    assert_eq!(fields.next(), Some("from-execution"));
    assert_ne!(fields.next(), Some("configured-id"));
    assert_eq!(fields.next(), Some("passed"));

    fzz(&directory)
        .env("PARENT_VALUE", "inherited")
        .args(["run", "failure"])
        .assert()
        .failure();
    let failure_hook = fs::read_to_string(directory.join("failure.txt")).unwrap();
    let mut fields = failure_hook.split('|');
    assert_eq!(fields.next(), Some("from-execution"));
    assert_ne!(fields.next(), Some("configured-id"));
    assert_eq!(fields.next(), Some("failed"));

    fs::remove_dir_all(directory).unwrap();
}

#[test]
fn invalid_execution_environment_fails_before_running_jobs_without_echoing_values() {
    let directory = fixture("invalid");
    fs::write(
        directory.join(".watch.yaml"),
        "on:\n  change: '**/*'\nexecution:\n  env:\n    BAD-NAME: secret-value\njobs:\n  - name: run\n    run: 'touch ran'\n",
    )
    .expect("write config");

    fzz(&directory)
        .arg("check")
        .assert()
        .failure()
        .stdout(predicates::str::contains(
            "Invalid environment variable name",
        ))
        .stdout(predicates::str::contains("secret-value").not());
    assert!(!directory.join("ran").exists());
    fs::remove_dir_all(directory).unwrap();
}
