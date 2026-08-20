use assert_cmd::cargo;
use predicates::prelude::*;
use pretty_assertions::assert_eq;
use std::fs;
use std::time::{SystemTime, UNIX_EPOCH};

/// TASK-0098: `fzz migrate` is the explicit, atomic, idempotent rewrite of
/// accepted legacy configuration into the preferred `jobs:` form.
/// `fzz init --migrate` is removed (V2 breaking cleanup, no deprecated path).
fn scratch(label: &str) -> std::path::PathBuf {
    let unique = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .expect("clock should be after epoch")
        .as_nanos();
    // Nanos can collide when two `cargo test` invocations overlap (watcher
    // generations, CI steps); the PID disambiguates concurrent processes.
    // Pre-remove defends against stale dirs from earlier crashed runs.
    let directory = std::env::temp_dir().join(format!(
        "funzzy-migrate-{}-{}-{}",
        std::process::id(),
        unique,
        label
    ));
    let _ = fs::remove_dir_all(&directory);
    fs::create_dir(&directory).expect("failed to create test directory");
    directory
}

#[test]
fn it_migrates_legacy_config_and_names_the_file() {
    let directory = scratch("legacy");
    let config = directory.join(".watch.yaml");
    fs::write(
        &config,
        "# project tasks\n\n- name: test\n  run: cargo test\n  run_on_init: true\n",
    )
    .expect("failed to create legacy config");

    cargo::cargo_bin_cmd!("funzzy")
        .current_dir(&directory)
        .env("FUNZZY_COLORED", "false")
        .env("_TEST_FUNZZY_COLORED", "false")
        .arg("migrate")
        .assert()
        .success()
        .stdout("Funzzy: .watch.yaml migrated to the jobs: form\n");

    assert_eq!(
        fs::read_to_string(&config).expect("failed to read migrated config"),
        "# project tasks\n\njobs:\n  - name: test\n    run: cargo test\n    run_on_init: true\n"
    );

    fs::remove_dir_all(directory).expect("failed to remove test directory");
}

#[test]
fn second_migration_is_a_byte_identical_noop() {
    let directory = scratch("idempotent");
    let config = directory.join(".watch.yaml");
    fs::write(
        &config,
        "# project tasks\n\n- name: test\n  run: cargo test\n  run_on_init: true\n",
    )
    .expect("failed to create legacy config");

    let _ = cargo::cargo_bin_cmd!("funzzy")
        .current_dir(&directory)
        .env("FUNZZY_COLORED", "false")
        .arg("migrate")
        .output()
        .unwrap();
    let migrated = fs::read_to_string(&config).unwrap();

    // Idempotent: exit 0, no-op message, bytes untouched.
    cargo::cargo_bin_cmd!("funzzy")
        .current_dir(&directory)
        .env("FUNZZY_COLORED", "false")
        .arg("migrate")
        .assert()
        .success()
        .stdout("Funzzy: .watch.yaml is already in the preferred jobs: form\n");
    assert_eq!(fs::read_to_string(&config).unwrap(), migrated);

    fs::remove_dir_all(directory).expect("failed to remove test directory");
}

#[test]
fn migrate_honors_global_config_selection() {
    let directory = scratch("custom");
    let custom = directory.join("legacy.yml");
    fs::write(&custom, "tasks:\n  - name: a\n    run: echo a\n")
        .expect("failed to create custom config");

    cargo::cargo_bin_cmd!("funzzy")
        .current_dir(&directory)
        .env("FUNZZY_COLORED", "false")
        .env("_TEST_FUNZZY_COLORED", "false")
        .args(["migrate", "-c", "legacy.yml"])
        .assert()
        .success()
        .stdout("Funzzy: legacy.yml migrated to the jobs: form\n");

    assert_eq!(
        fs::read_to_string(&custom).unwrap(),
        "jobs:\n  - name: a\n    run: echo a\n"
    );

    fs::remove_dir_all(directory).expect("failed to remove test directory");
}

#[test]
fn missing_file_is_an_operational_error() {
    let directory = scratch("missing");

    cargo::cargo_bin_cmd!("funzzy")
        .current_dir(&directory)
        .env("FUNZZY_COLORED", "false")
        .arg("migrate")
        .assert()
        .failure()
        .code(1)
        .stdout(predicate::str::is_empty())
        .stderr(predicate::str::contains("Failed to migrate .watch.yaml"));

    fs::remove_dir_all(directory).expect("failed to remove test directory");
}

#[test]
fn malformed_input_fails_without_touching_bytes() {
    let directory = scratch("malformed");
    let config = directory.join(".watch.yaml");
    fs::write(&config, "name: [unclosed\n").expect("seed malformed config");

    cargo::cargo_bin_cmd!("funzzy")
        .current_dir(&directory)
        .env("FUNZZY_COLORED", "false")
        .arg("migrate")
        .assert()
        .failure()
        .code(1)
        .stdout(predicate::str::is_empty())
        .stderr(predicate::str::contains("Failed to migrate .watch.yaml"));

    // Failed migration leaves the original bytes unchanged.
    assert_eq!(fs::read_to_string(&config).unwrap(), "name: [unclosed\n");

    fs::remove_dir_all(directory).expect("failed to remove test directory");
}

#[test]
fn init_migrate_flag_is_removed() {
    let directory = scratch("removed-flag");

    // `init --migrate` is no longer a valid invocation: exit 2 on stderr.
    cargo::cargo_bin_cmd!("funzzy")
        .current_dir(&directory)
        .args(["init", "--migrate"])
        .assert()
        .failure()
        .code(2)
        .stderr(predicate::str::contains("--migrate"));

    fs::remove_dir_all(directory).expect("failed to remove test directory");
}
