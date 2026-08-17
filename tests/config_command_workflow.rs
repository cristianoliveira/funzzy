//! TASK-0099: black-box proof for the simplified configuration workflow.
//! Exercises the installed `fzz` binary in isolated workspaces — create,
//! export, inspect schema, validate, and migrate are distinct observable
//! responsibilities.

use assert_cmd::cargo;
use std::fs;
use std::path::{Path, PathBuf};
use std::time::{SystemTime, UNIX_EPOCH};

const PROFILES: [&str; 4] = ["comprehensive", "minimal", "parallel", "agent"];

fn fixture(label: &str) -> PathBuf {
    let unique = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .expect("clock after epoch")
        .as_nanos();
    let directory = std::env::temp_dir().join(format!(
        "funzzy-config-workflow-{}-{}-{}",
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
        .env("_TEST_FUNZZY_COLORED", "false");
    command
}

fn entries(directory: &Path) -> Vec<String> {
    let mut names: Vec<String> = fs::read_dir(directory)
        .expect("read fixture")
        .map(|entry| {
            entry
                .expect("read entry")
                .file_name()
                .to_string_lossy()
                .into_owned()
        })
        .collect();
    names.sort();
    names
}

#[test]
fn every_profile_is_deterministic_valid_and_byte_equal_across_destinations() {
    for profile in PROFILES {
        let first = fixture(&format!("{profile}-first"));
        let second = fixture(&format!("{profile}-second"));
        let export = fixture(&format!("{profile}-export"));

        fzz(&first)
            .args(["init", "--template", profile])
            .assert()
            .success();
        fzz(&second)
            .args(["init", "--template", profile])
            .assert()
            .success();

        let first_bytes = fs::read(first.join(".watch.yaml")).expect("first init output");
        let second_bytes = fs::read(second.join(".watch.yaml")).expect("second init output");
        assert_eq!(
            first_bytes, second_bytes,
            "{profile}: deterministic init bytes"
        );

        // `config example` is stdout-only: no prose on stderr, no filesystem
        // side effect, and bytes exactly equal init's selected artifact.
        let before = entries(&export);
        let example = fzz(&export)
            .args(["config", "example", profile])
            .output()
            .expect("run config example");
        assert!(example.status.success(), "{profile}: example succeeds");
        assert!(example.stderr.is_empty(), "{profile}: no prose on stderr");
        assert_eq!(entries(&export), before, "{profile}: no filesystem write");
        assert_eq!(example.stdout, first_bytes, "{profile}: destination parity");

        // Every artifact is accepted by the installed production validator.
        fzz(&first).arg("check").assert().success();

        for directory in [first, second, export] {
            fs::remove_dir_all(directory).expect("remove fixture");
        }
    }

    // Omitted --template remains an alias for the explicit comprehensive
    // profile; this protects `fzz init && fzz`.
    let implicit = fixture("implicit-comprehensive");
    let explicit = fixture("explicit-comprehensive");
    fzz(&implicit).arg("init").assert().success();
    fzz(&explicit)
        .args(["init", "--template", "comprehensive"])
        .assert()
        .success();
    assert_eq!(
        fs::read(implicit.join(".watch.yaml")).unwrap(),
        fs::read(explicit.join(".watch.yaml")).unwrap()
    );
    fs::remove_dir_all(implicit).unwrap();
    fs::remove_dir_all(explicit).unwrap();
}

#[test]
fn init_refuses_existing_destination_for_every_profile() {
    for profile in PROFILES {
        let directory = fixture(&format!("refuse-{profile}"));
        let config = directory.join(".watch.yaml");
        fs::write(&config, b"# existing bytes\n").expect("seed config");

        fzz(&directory)
            .args(["init", "--template", profile])
            .assert()
            .failure()
            .code(1);
        assert_eq!(
            fs::read(&config).unwrap(),
            b"# existing bytes\n",
            "{profile}: refusal never mutates"
        );
        fs::remove_dir_all(directory).unwrap();
    }
}

#[test]
fn invalid_profile_is_a_corrective_clap_usage_error() {
    let directory = fixture("invalid-profile");
    let output = fzz(&directory)
        .args(["init", "--template", "bogus"])
        .output()
        .expect("run invalid init");
    assert_eq!(output.status.code(), Some(2));
    assert!(output.stdout.is_empty());
    let error = String::from_utf8_lossy(&output.stderr);
    assert!(error.contains("invalid value 'bogus'"), "{error}");
    for profile in PROFILES {
        assert!(
            error.contains(profile),
            "correction names {profile}: {error}"
        );
    }
    assert!(
        entries(&directory).is_empty(),
        "usage error creates no file"
    );
    fs::remove_dir_all(directory).unwrap();
}

#[test]
fn config_schema_is_machine_pure_and_side_effect_free() {
    let directory = fixture("schema");
    let before = entries(&directory);
    let output = fzz(&directory)
        .args(["config", "schema", "--format", "json"])
        .output()
        .expect("run schema");
    assert!(output.status.success());
    assert!(output.stderr.is_empty(), "schema diagnostics must be empty");
    assert_eq!(entries(&directory), before, "schema must not write files");

    // Parsing the complete stdout proves no prose surrounds the JSON payload.
    let schema: serde_json::Value =
        serde_json::from_slice(&output.stdout).expect("stdout is exactly one JSON document");
    assert_eq!(schema["type"], "object");
    assert!(schema["properties"]["jobs"].is_object());

    fs::remove_dir_all(directory).unwrap();
}

#[test]
fn help_exposes_create_only_init_and_explicit_migrate() {
    let directory = fixture("help");
    let init = fzz(&directory)
        .args(["init", "--help"])
        .output()
        .expect("init help");
    let init_help = String::from_utf8_lossy(&init.stdout);
    assert!(init.status.success());
    assert!(init_help.contains("--template"));
    assert!(init_help.contains("never overwrites"));
    assert!(!init_help.contains("--migrate"));
    for profile in PROFILES {
        assert!(init_help.contains(profile), "init help names {profile}");
    }

    let migrate = fzz(&directory)
        .args(["migrate", "--help"])
        .output()
        .expect("migrate help");
    let migrate_help = String::from_utf8_lossy(&migrate.stdout);
    assert!(migrate.status.success());
    assert!(migrate_help.contains("ordered 'jobs:' form"));
    assert!(migrate_help.contains("-c"));
    assert!(migrate_help.contains("--config"));

    let top = fzz(&directory).arg("--help").output().expect("top help");
    let top_help = String::from_utf8_lossy(&top.stdout);
    assert!(top_help.contains("migrate"));

    fs::remove_dir_all(directory).unwrap();
}

#[test]
fn public_guides_present_one_create_export_validate_and_migrate_path() {
    let root = Path::new(env!("CARGO_MANIFEST_DIR"));
    let read = |path: &str| fs::read_to_string(root.join(path)).expect("read guide");
    let readme = read("README.md");
    let usage = read("docs/USAGE.md");
    let migration = read("docs/MIGRATION.md");

    for document in [&readme, &usage] {
        for command in [
            "fzz init",
            "fzz config example",
            "fzz config schema",
            "fzz check",
            "fzz migrate",
        ] {
            assert!(document.contains(command), "guide must present {command}");
        }
        assert!(
            !document.contains("fzz init --migrate"),
            "live guide must not advertise removed migration path"
        );
    }
    assert!(migration.contains("fzz migrate"));
    assert!(!migration.contains("fzz init --migrate"));
}
