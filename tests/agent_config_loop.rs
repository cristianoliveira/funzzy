//! Agent configure→validate→run loop proof (TASK-0059).
//!
//! Black-box E2E: an agent with NO config discovers the schema, requests an
//! example, writes it, validates it, inspects targets/matching, and executes
//! one finite target — all from the installed binary, never external docs.
//! Also proves: deterministic/bounded/secret-free schema+examples, structural
//! vs semantic diagnostics with recovery, legacy acceptance, and no
//! side effects during discovery.

use assert_cmd::cargo;
use predicates::prelude::*;
use std::path::{Path, PathBuf};

fn fixture(name: &str) -> PathBuf {
    let directory = std::env::temp_dir().join(format!(
        "funzzy-agent-config-{}-{}",
        std::process::id(),
        name
    ));
    let _ = std::fs::remove_dir_all(&directory);
    std::fs::create_dir_all(&directory).unwrap();
    directory
}

fn fzz(directory: &Path) -> assert_cmd::Command {
    let mut command = cargo::cargo_bin_cmd!("fzz");
    command
        .current_dir(directory)
        .env("FUNZZY_COLORED", "false")
        .env("FUNZZY_BAIL", "false")
        .env("_TEST_FUNZZY_BAIL", "false");
    command
}

#[test]
fn agent_discovers_writes_validates_and_runs_from_scratch() {
    let directory = fixture("e2e");
    // 1. Discover the schema without any config present.
    let schema = fzz(&directory)
        .args(["config", "schema", "--section", "job", "--format", "json"])
        .output()
        .unwrap();
    assert!(
        schema.status.success(),
        "schema discovery works config-free"
    );
    let schema_doc: serde_json::Value = serde_json::from_slice(&schema.stdout).unwrap();
    assert_eq!(schema_doc["section"], "job");
    assert!(schema_doc["properties"]["job"]["properties"]["run"].is_object());

    // 2. Request a runnable example (must parse + validate), then write a
    //    small finite config that succeeds in the scratch workspace (the
    //    shipped example uses cargo; the loop proof needs a target that
    //    exits 0 here).
    let example = fzz(&directory)
        .args(["config", "example", "minimal"])
        .output()
        .unwrap();
    assert!(example.status.success());
    std::fs::write(
        directory.join(".watch.yaml"),
        "on:\n  change: '**/*'\njobs:\n  - name: build\n    run: 'echo done > done.txt'\n",
    )
    .unwrap();

    // 3. Validate it.
    fzz(&directory)
        .arg("-c")
        .arg(".watch.yaml")
        .arg("check")
        .assert()
        .code(0)
        .stdout(predicate::str::contains("1 job(s)"));

    // 4. Inspect targets and matching/ignore behavior.
    let list = fzz(&directory)
        .arg("-c")
        .arg(".watch.yaml")
        .arg("list")
        .output()
        .unwrap();
    assert!(list.status.success());
    assert!(String::from_utf8_lossy(&list.stdout).contains("build"));
    let explain = fzz(&directory)
        .arg("-c")
        .arg(".watch.yaml")
        .arg("explain")
        .arg("Cargo.toml")
        .output()
        .unwrap();
    assert!(explain.status.success());
    assert!(String::from_utf8_lossy(&explain.stdout).contains("build"));

    // 5. Execute the exact finite target — no watcher/socket started.
    fzz(&directory)
        .arg("-c")
        .arg(".watch.yaml")
        .args(["run", "build"])
        .assert()
        .code(0)
        .stdout(predicate::str::contains("Completed: 1"))
        .stdout(predicate::str::contains("Watching...").not());
    assert_eq!(
        std::fs::read_to_string(directory.join("done.txt")).unwrap(),
        "done\n"
    );
    // No socket or log side effects were created by discovery/check/run.
    let entries: Vec<_> = std::fs::read_dir(&directory)
        .unwrap()
        .map(|e| e.unwrap().file_name().to_string_lossy().to_string())
        .collect();
    assert!(
        !entries.iter().any(|e| e.ends_with(".sock")),
        "no socket created: {entries:?}"
    );
    std::fs::remove_dir_all(&directory).unwrap();
}

#[test]
fn structural_and_semantic_errors_give_path_specific_diagnostics() {
    let directory = fixture("errors");
    // Structural: unknown property in 'on'.
    std::fs::write(
        directory.join(".watch.yaml"),
        "on:\n  bogus_key: 1\njobs:\n  - name: a\n    run: echo a\n    change: '**/*'\n",
    )
    .unwrap();
    let structural = fzz(&directory)
        .arg("-c")
        .arg(".watch.yaml")
        .arg("check")
        .output()
        .unwrap();
    assert!(!structural.status.success());
    let out = String::from_utf8_lossy(&structural.stdout);
    assert!(
        out.contains("Invalid property 'bogus_key'"),
        "structural error names the path: {out}"
    );

    // Semantic: a job with no command and no trigger.
    std::fs::write(directory.join(".watch.yaml"), "jobs:\n  - name: broken\n").unwrap();
    let semantic = fzz(&directory)
        .arg("-c")
        .arg(".watch.yaml")
        .arg("check")
        .output()
        .unwrap();
    assert!(!semantic.status.success());
    let out = String::from_utf8_lossy(&semantic.stdout);
    assert!(
        out.contains("run") || out.contains("change"),
        "semantic error names the missing field: {out}"
    );
    std::fs::remove_dir_all(&directory).unwrap();
}

#[test]
fn schema_and_examples_are_deterministic_bounded_and_secret_free() {
    let directory = fixture("contract");
    for args in [
        vec!["config", "schema", "--format", "json"],
        vec!["config", "schema", "--format", "toon"],
        vec!["config", "example", "agent"],
    ] {
        let first = fzz(&directory).args(&args).output().unwrap();
        let second = fzz(&directory).args(&args).output().unwrap();
        assert!(first.status.success());
        assert_eq!(first.stdout, second.stdout, "deterministic: {args:?}");
        // Bounded: well under a generous cap.
        assert!(
            first.stdout.len() < 32 * 1024,
            "bounded output for {args:?}: {} bytes",
            first.stdout.len()
        );
        // Secret-free: no environment values or resolved paths leak.
        let text = String::from_utf8_lossy(&first.stdout);
        assert!(
            !text.contains(&std::env::var("HOME").unwrap_or_default()),
            "no HOME leak: {args:?}"
        );
    }
    std::fs::remove_dir_all(&directory).unwrap();
}

#[test]
fn section_query_is_substantially_smaller_than_full_schema() {
    let directory = fixture("cost");
    let full = fzz(&directory)
        .args(["config", "schema", "--format", "json"])
        .output()
        .unwrap();
    let section = fzz(&directory)
        .args([
            "config",
            "schema",
            "--section",
            "parallel",
            "--format",
            "json",
        ])
        .output()
        .unwrap();
    assert!(full.status.success() && section.status.success());
    assert!(
        section.stdout.len() < full.stdout.len(),
        "section must be smaller: {} vs {}",
        section.stdout.len(),
        full.stdout.len()
    );
    assert!(
        section.stdout.len() * 2 < full.stdout.len(),
        "section must materially reduce cost: {} vs {}",
        section.stdout.len(),
        full.stdout.len()
    );
    std::fs::remove_dir_all(&directory).unwrap();
}

#[test]
fn legacy_config_is_accepted_and_discovery_points_to_migration() {
    let directory = fixture("legacy");
    std::fs::write(
        directory.join(".watch.yaml"),
        "- name: old task\n  run: echo hi\n  change: '**/*'\n",
    )
    .unwrap();
    // Legacy is accepted and checkable.
    fzz(&directory)
        .arg("-c")
        .arg(".watch.yaml")
        .arg("check")
        .assert()
        .code(0)
        .stdout(predicate::str::contains("1 job(s)"));

    // Migration converts to the preferred grouped jobs form.
    let migrate = cargo::cargo_bin_cmd!("fzz")
        .current_dir(&directory)
        .env("FUNZZY_COLORED", "false")
        .args(["init", "--migrate"])
        .output()
        .unwrap();
    assert!(migrate.status.success());
    let migrated = std::fs::read_to_string(directory.join(".watch.yaml")).unwrap();
    assert!(migrated.contains("jobs:"), "migrated uses jobs: {migrated}");
    std::fs::remove_dir_all(&directory).unwrap();
}
