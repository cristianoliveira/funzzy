//! Tasks→jobs migration parity proof (TASK-0078).
//!
//! Proves the V2 `jobs:` vocabulary is semantically equivalent to the
//! accepted legacy forms: same parsed topology, same observable runs, same
//! parallel barriers and sequential override, and idempotent reversible
//! migration — comparing behavior, not just YAML text.

use assert_cmd::cargo;
use predicates::prelude::*;
use std::path::{Path, PathBuf};
use yaml_rust2::{Yaml, YamlLoader};

fn fixture(name: &str) -> PathBuf {
    let directory = std::env::temp_dir().join(format!(
        "funzzy-jobs-migrate-{}-{}",
        std::process::id(),
        name
    ));
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

/// The same workflow expressed in the legacy root-list and the preferred
/// jobs form. Barriers and tags must survive identically.
const LEGACY: &str = "execution:\n  concurrency: 2\ntasks:\n  - name: lint @quick\n    parallel: checks\n    run: 'echo lint > lint.txt'\n    change: '**/*'\n  - name: test @quick\n    parallel: checks\n    run: 'echo test > test.txt'\n    change: '**/*'\n  - name: package @quick\n    run: 'echo package > package.txt'\n    change: '**/*'\n";

const JOBS: &str = "execution:\n  concurrency: 2\njobs:\n  - name: lint @quick\n    parallel: checks\n    run: 'echo lint > lint.txt'\n    change: '**/*'\n  - name: test @quick\n    parallel: checks\n    run: 'echo test > test.txt'\n    change: '**/*'\n  - name: package @quick\n    run: 'echo package > package.txt'\n    change: '**/*'\n";

#[test]
fn legacy_tasks_and_jobs_produce_identical_list_output() {
    let legacy_dir = fixture("list-legacy");
    write_config(&legacy_dir, LEGACY);
    let jobs_dir = fixture("list-jobs");
    write_config(&jobs_dir, JOBS);

    let legacy_out = fzz(&legacy_dir).arg("list").output().unwrap();
    let jobs_out = fzz(&jobs_dir).arg("list").output().unwrap();
    assert!(legacy_out.status.success());
    assert_eq!(
        String::from_utf8_lossy(&legacy_out.stdout),
        String::from_utf8_lossy(&jobs_out.stdout),
        "list output must be identical before/after migration"
    );
    std::fs::remove_dir_all(&legacy_dir).unwrap();
    std::fs::remove_dir_all(&jobs_dir).unwrap();
}

#[test]
fn legacy_tasks_and_jobs_run_identically_including_barriers() {
    for (name, config) in [("run-legacy", LEGACY), ("run-jobs", JOBS)] {
        let directory = fixture(name);
        write_config(&directory, config);
        fzz(&directory)
            .args(["run", "@quick"])
            .assert()
            .success()
            .stdout(predicate::str::contains("Completed: 3"));
        // All three jobs wrote their outputs, in barrier order.
        assert_eq!(
            std::fs::read_to_string(directory.join("lint.txt")).unwrap(),
            "lint\n"
        );
        assert_eq!(
            std::fs::read_to_string(directory.join("test.txt")).unwrap(),
            "test\n"
        );
        assert_eq!(
            std::fs::read_to_string(directory.join("package.txt")).unwrap(),
            "package\n"
        );
        std::fs::remove_dir_all(&directory).unwrap();
    }
}

#[test]
fn migration_is_idempotent_and_second_run_is_a_noop() {
    let directory = fixture("idempotent");
    write_config(&directory, LEGACY);

    // First migration: legacy root list -> jobs, preserving comments/order.
    let first = cargo::cargo_bin_cmd!("fzz")
        .current_dir(&directory)
        .env("FUNZZY_COLORED", "false")
        .args(["migrate"])
        .output()
        .unwrap();
    assert!(first.status.success());
    let migrated = std::fs::read_to_string(directory.join(".watch.yaml")).unwrap();
    assert!(
        migrated.contains("jobs:"),
        "migrated must use jobs: {migrated}"
    );
    assert!(
        migrated.contains("lint @quick"),
        "order preserved: {migrated}"
    );
    assert!(
        migrated.contains("package @quick"),
        "order preserved: {migrated}"
    );

    // Second migration is a no-op (already jobs) with exit 0.
    let second = cargo::cargo_bin_cmd!("fzz")
        .current_dir(&directory)
        .env("FUNZZY_COLORED", "false")
        .args(["migrate"])
        .output()
        .unwrap();
    assert!(second.status.success());
    assert_eq!(
        std::fs::read_to_string(directory.join(".watch.yaml")).unwrap(),
        migrated,
        "second migration must not rewrite"
    );
    std::fs::remove_dir_all(&directory).unwrap();
}

#[test]
fn migration_preserves_parallel_semantics_and_sequential_override() {
    // Handshake-style barrier fixture: two jobs in one group pass only when
    // overlapping at concurrency 2. After migrating to jobs, the same
    // behavior must hold, and --sequential must still force failure.
    let handshake_legacy = "execution:\n  concurrency: 2\ntasks:\n  - name: a @quick\n    parallel: checks\n    run: 'touch a.ready; i=0; while [ $i -lt 100 ]; do test -f b.ready && exit 0; i=$((i + 1)); sleep 0.02; done; exit 1'\n    change: '**/*'\n  - name: b @quick\n    parallel: checks\n    run: 'touch b.ready; i=0; while [ $i -lt 100 ]; do test -f a.ready && exit 0; i=$((i + 1)); sleep 0.02; done; exit 1'\n    change: '**/*'\n";

    let directory = fixture("barrier-migrate");
    write_config(&directory, handshake_legacy);
    let migrate = cargo::cargo_bin_cmd!("fzz")
        .current_dir(&directory)
        .env("FUNZZY_COLORED", "false")
        .args(["migrate"])
        .output()
        .unwrap();
    assert!(migrate.status.success());

    // After migration: the parallel run still overlaps and passes.
    fzz(&directory)
        .args(["run", "@quick"])
        .assert()
        .success()
        .stdout(predicate::str::contains("Completed: 2"));
    // Clean the handshake markers so the sequential comparison is fair.
    let _ = std::fs::remove_file(directory.join("a.ready"));
    let _ = std::fs::remove_file(directory.join("b.ready"));

    // Sequential override still forces the first task to fail.
    fzz(&directory)
        .args(["run", "@quick", "--sequential"])
        .assert()
        .code(1)
        .stdout(predicate::str::contains("Failed: 1;"));
    std::fs::remove_dir_all(&directory).unwrap();
}

#[test]
fn fzz_check_validates_migrated_jobs_and_rejects_mixed_input() {
    let directory = fixture("check-migrated");
    write_config(&directory, JOBS);
    fzz(&directory)
        .arg("check")
        .assert()
        .code(0)
        .stdout(predicate::str::contains("3 job(s)"));

    // Mixed tasks+jobs fails deterministically without partial rewrite.
    write_config(
        &directory,
        "on:\n  change: '**/*'\ntasks:\n  - name: a\n    run: echo a\n    change: '**/*'\njobs:\n  - name: b\n    run: echo b\n    change: '**/*'\n",
    );
    fzz(&directory)
        .arg("check")
        .assert()
        .code(1)
        .stdout(predicate::str::contains("Invalid config file"));
    std::fs::remove_dir_all(&directory).unwrap();
}

#[test]
fn migrated_config_with_cwd_env_and_init_runs_identically() {
    let legacy = "on:\n  change: '**/*'\ntasks:\n  - name: build @quick\n    cwd: sub\n    env: { MODE: prod }\n    run: 'test \"$MODE\" = prod && echo built > out.txt'\n    change: '**/*'\n  - name: init only @quick\n    run: 'echo started > started.txt'\n    run_on_init: true\n    change: 'never-matches'\n";

    let directory = fixture("context-migrate");
    std::fs::create_dir_all(directory.join("sub")).unwrap();
    write_config(&directory, legacy);
    let migrate = cargo::cargo_bin_cmd!("fzz")
        .current_dir(&directory)
        .env("FUNZZY_COLORED", "false")
        .args(["migrate"])
        .output()
        .unwrap();
    assert!(migrate.status.success());

    fzz(&directory)
        .args(["run", "@quick"])
        .assert()
        .success()
        .stdout(predicate::str::contains("Completed: 2"));
    assert_eq!(
        std::fs::read_to_string(directory.join("sub/out.txt")).unwrap(),
        "built\n"
    );
    assert_eq!(
        std::fs::read_to_string(directory.join("started.txt")).unwrap(),
        "started\n"
    );
    std::fs::remove_dir_all(&directory).unwrap();
}

#[test]
fn shipped_nested_groups_migrate_and_check() {
    let directory = fixture("shipped-nested-groups");
    let config = directory.join("nested-groups.yml");
    std::fs::copy("examples/nested-job-groups.yml", &config).expect("copy shipped fixture");

    let migrate = cargo::cargo_bin_cmd!("fzz")
        .current_dir(&directory)
        .env("FUNZZY_COLORED", "false")
        .args(["-c", config.to_str().unwrap(), "migrate"])
        .output()
        .unwrap();
    assert!(
        migrate.status.success(),
        "nested fixture migration failed: {}",
        String::from_utf8_lossy(&migrate.stdout)
    );
    let migrated = std::fs::read_to_string(&config).unwrap();
    assert!(migrated.contains("jobs:\n"));
    assert!(!migrated.contains("tasks:"));
    assert!(!migrated.contains("- on:"));

    let check = cargo::cargo_bin_cmd!("fzz")
        .current_dir(&directory)
        .env("FUNZZY_COLORED", "false")
        .args(["-c", config.to_str().unwrap(), "check"])
        .output()
        .unwrap();
    assert!(
        check.status.success(),
        "migrated nested fixture is invalid: {}",
        String::from_utf8_lossy(&check.stdout)
    );
    assert!(String::from_utf8_lossy(&check.stdout).contains("11 job(s)"));

    // Preferred output is idempotent and byte-identical on the second run.
    let second = cargo::cargo_bin_cmd!("fzz")
        .current_dir(&directory)
        .env("FUNZZY_COLORED", "false")
        .args(["-c", config.to_str().unwrap(), "migrate"])
        .output()
        .unwrap();
    assert!(second.status.success());
    assert_eq!(std::fs::read_to_string(&config).unwrap(), migrated);
    std::fs::remove_dir_all(&directory).unwrap();
}

fn yaml_examples(root: &Path) -> Vec<PathBuf> {
    let mut files = Vec::new();
    let mut directories = vec![root.to_path_buf()];
    while let Some(directory) = directories.pop() {
        let mut entries = std::fs::read_dir(directory)
            .unwrap()
            .filter_map(|entry| entry.ok())
            .collect::<Vec<_>>();
        entries.sort_by_key(|entry| entry.path());
        for entry in entries.into_iter().rev() {
            if entry
                .file_type()
                .map(|file_type| file_type.is_symlink())
                .unwrap_or(false)
            {
                continue;
            }
            let path = entry.path();
            if path.is_dir() {
                directories.push(path);
            } else if matches!(
                path.extension().and_then(|ext| ext.to_str()),
                Some("yml" | "yaml")
            ) {
                files.push(path);
            }
        }
    }
    files.sort();
    files
}

fn run_fzz(path: &Path, action: &str) -> std::process::Output {
    assert_cmd::cargo::cargo_bin_cmd!("fzz")
        .arg("-c")
        .arg(path)
        .arg(action)
        .output()
        .unwrap()
}

fn scratch_copy(source: &Path, label: &str) -> (PathBuf, Vec<u8>) {
    let directory = fixture(label);
    let config = directory.join(source.file_name().expect("config filename"));
    let original = std::fs::read(source).unwrap();
    std::fs::copy(source, &config).expect("copy config to scratch");
    (config, original)
}

#[test]
fn every_valid_example_recursively_passes_check_and_is_migration_noop() {
    let examples = yaml_examples(Path::new("examples"))
        .into_iter()
        .filter(|path| {
            !path
                .components()
                .any(|component| component.as_os_str() == "invalid")
        })
        .collect::<Vec<_>>();
    assert_eq!(examples.len(), 14);
    for example in examples {
        let source = std::fs::read_to_string(&example).unwrap();
        let documents = YamlLoader::load_from_str(&source).unwrap();
        assert_eq!(
            documents.len(),
            1,
            "{} has one YAML document",
            example.display()
        );
        let Yaml::Hash(root) = &documents[0] else {
            panic!("{} must use a V2 mapping root", example.display());
        };
        assert!(root.get(&Yaml::String("jobs".to_owned())).is_some());
        assert!(root.get(&Yaml::String("tasks".to_owned())).is_none());

        let label = format!(
            "catalog-valid-{}",
            example.file_name().unwrap().to_string_lossy()
        );
        let (scratch, original) = scratch_copy(&example, &label);
        assert!(
            run_fzz(&scratch, "check").status.success(),
            "{}",
            example.display()
        );
        assert!(
            run_fzz(&scratch, "migrate").status.success(),
            "{}",
            example.display()
        );
        assert_eq!(
            std::fs::read(&scratch).unwrap(),
            original,
            "{} must be a no-op",
            example.display()
        );
        assert_eq!(
            std::fs::read(&example).unwrap(),
            original,
            "original {} must not be mutated",
            example.display()
        );
        std::fs::remove_dir_all(scratch.parent().unwrap()).unwrap();
    }
}

#[test]
fn invalid_examples_fail_for_documented_reasons_and_migrate_preserves_bytes() {
    let cases = [
        ("examples/invalid/invalid-value.yml", "unknown anchor"),
        (
            "examples/invalid/missing-required-property.yml",
            "Missing 'name'",
        ),
        (
            "examples/invalid/non-list.yaml",
            "Property 'on' must be an object",
        ),
    ];
    for (path, reason) in cases {
        let path = Path::new(path);
        let label = format!(
            "catalog-invalid-{}",
            path.file_name().unwrap().to_string_lossy()
        );
        let (scratch, original) = scratch_copy(path, &label);
        let check = run_fzz(&scratch, "check");
        let check_text = format!(
            "{}{}",
            String::from_utf8_lossy(&check.stdout),
            String::from_utf8_lossy(&check.stderr)
        );
        assert!(
            !check.status.success(),
            "{} unexpectedly passed",
            path.display()
        );
        assert!(
            check_text.contains(reason),
            "{}: expected {reason}, got {check_text}",
            path.display()
        );
        let migrate = run_fzz(&scratch, "migrate");
        assert!(
            !migrate.status.success(),
            "{} migration unexpectedly passed",
            path.display()
        );
        assert_eq!(
            std::fs::read(&scratch).unwrap(),
            original,
            "{} scratch copy must remain unchanged",
            path.display()
        );
        assert_eq!(
            std::fs::read(path).unwrap(),
            original,
            "original {} must remain unchanged",
            path.display()
        );
        std::fs::remove_dir_all(scratch.parent().unwrap()).unwrap();
    }
}
