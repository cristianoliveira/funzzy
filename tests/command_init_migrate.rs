use assert_cmd::cargo;
use pretty_assertions::assert_eq;
use std::fs;
use std::time::{SystemTime, UNIX_EPOCH};

#[test]
fn it_migrates_legacy_config_with_init_migrate() {
    let unique = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .expect("clock should be after epoch")
        .as_nanos();
    // Nanos alone can collide when two `cargo test` invocations overlap
    // (watcher generations, CI steps) and read the same clock tick; the PID
    // disambiguates concurrent processes. Pre-remove defends against stale
    // dirs from earlier crashed runs.
    let directory = std::env::temp_dir().join(format!(
        "funzzy-init-migrate-{}-{}",
        std::process::id(),
        unique
    ));
    let _ = fs::remove_dir_all(&directory);
    fs::create_dir(&directory).expect("failed to create test directory");
    let config = directory.join(".watch.yaml");
    fs::write(
        &config,
        "# project tasks\n\n- name: test\n  run: cargo test\n  run_on_init: true\n",
    )
    .expect("failed to create legacy config");

    let mut command = cargo::cargo_bin_cmd!("funzzy");
    command
        .current_dir(&directory)
        .env("FUNZZY_COLORED", "false")
        .env("_TEST_FUNZZY_COLORED", "false")
        .args(["init", "--migrate"])
        .assert()
        .success()
        .stdout("Funzzy: Configuration file migrated successfully!\n");

    assert_eq!(
        fs::read_to_string(&config).expect("failed to read migrated config"),
        "# project tasks\n\ntasks:\n  - name: test\n    run: cargo test\n    run_on_init: true\n"
    );

    fs::remove_dir_all(directory).expect("failed to remove test directory");
}
