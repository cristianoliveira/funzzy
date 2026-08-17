//! Black-box proof of the comprehensive commented init template (TASK-0095):
//! a freshly created `.watch.yaml` is deterministic, accepted by `fzz check`,
//! immediately runnable in an empty directory (no language/toolchain
//! dependency), and its comments cover the canonical option catalog without
//! advertising unsupported properties. Documented values parse; invalid
//! alternatives fail deterministically. Example profiles never inherit the
//! human-commented init output.

#[cfg(feature = "test-integration")]
use std::io::{Read, Write};
use std::path::{Path, PathBuf};
use std::process::Command;
#[cfg(feature = "test-integration")]
use std::process::Stdio;

#[path = "./common/lib.rs"]
mod setup;

/// Fresh empty scratch dir per run — never the repo, never shared across
/// processes or tests.
fn empty_dir(label: &str) -> PathBuf {
    let dir = std::env::temp_dir().join(format!(
        "funzzy-init-proof-{}-{}",
        std::process::id(),
        label
    ));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).expect("failed to create scratch dir");
    dir
}

fn fzz() -> Command {
    Command::new(env!("CARGO_BIN_EXE_fzz"))
}

fn init_in(dir: &Path) -> std::process::Output {
    fzz()
        .arg("init")
        .current_dir(dir)
        .output()
        .expect("failed to run fzz init")
}

fn check_ok(dir: &Path, config: &str) -> std::process::Output {
    let file = dir.join("probe.yaml");
    std::fs::write(&file, config).expect("failed to write probe config");
    fzz()
        .args(["check", "--config"])
        .arg(&file)
        .current_dir(dir)
        .output()
        .expect("failed to run fzz check")
}

/// Criterion 1: `fzz init` creates exactly one deterministic `.watch.yaml`,
/// `fzz check` accepts it, and a second init refuses overwrite without
/// mutating the file.
#[test]
fn init_is_deterministic_single_file_and_refuses_overwrite() {
    let dir = empty_dir("init-contract");

    let first = init_in(&dir);
    assert!(first.status.success(), "first init failed");
    let yaml_path = dir.join(".watch.yaml");
    assert!(yaml_path.exists(), "init must create .watch.yaml");
    assert_eq!(
        std::fs::read_dir(&dir)
            .expect("read dir")
            .filter(|e| e.as_ref().unwrap().file_name() == ".watch.yaml")
            .count(),
        1,
        "exactly one .watch.yaml expected"
    );
    let bytes = std::fs::read(&yaml_path).expect("read .watch.yaml");

    // Deterministic across runs in a different directory.
    let dir2 = empty_dir("init-contract-2");
    let second = init_in(&dir2);
    assert!(second.status.success(), "second init failed");
    assert_eq!(
        std::fs::read(dir2.join(".watch.yaml")).expect("read second"),
        bytes,
        "init bytes must be deterministic"
    );

    // `fzz check` accepts the generated file (same parser as the watcher).
    let check = fzz()
        .arg("check")
        .current_dir(&dir)
        .output()
        .expect("failed to run fzz check");
    assert!(
        check.status.success(),
        "generated config must pass fzz check: {}",
        String::from_utf8_lossy(&check.stderr)
    );

    // Second init refuses overwrite without mutation.
    let refused = init_in(&dir);
    assert!(
        !refused.status.success(),
        "second init must refuse overwrite"
    );
    assert_eq!(
        std::fs::read(&yaml_path).expect("re-read .watch.yaml"),
        bytes,
        "refused init must not mutate the file"
    );

    let _ = std::fs::remove_dir_all(&dir);
    let _ = std::fs::remove_dir_all(&dir2);
}

/// Criterion 2+3: starting the generated config runs the generic init example
/// with no Cargo/npm/language dependency, and creating a matching generic file
/// triggers the generic change example — the starter works immediately.
#[cfg(feature = "test-integration")]
#[test]
fn generated_config_runs_init_and_change_jobs() {
    let dir = empty_dir("init-watch");
    setup::serialized(|| {
        let init = init_in(&dir);
        assert!(init.status.success(), "init failed");

        // Log lives inside the watched dir; `ignore: '**/*.log'` keeps it from
        // triggering the change job in a loop.
        let log_path = dir.join(format!("proof-{}.log", std::process::id()));
        let log_file = std::fs::File::create(&log_path).expect("create log");
        let mut child = fzz()
            .current_dir(&dir)
            .stdout(Stdio::from(log_file))
            .spawn()
            .expect("failed to spawn fzz watcher");

        defer!({
            let _ = child.kill();
        });

        let mut output = String::new();
        // Init example: `run_on_init: true` hello job prints without any
        // language/toolchain present in the empty directory.
        wait_until!(
            {
                output.clear();
                let mut log = std::fs::File::open(&log_path).expect("open log");
                log.read_to_string(&mut output).expect("read log");
                output.contains("Funzzy hello world")
            },
            "generated hello job must run on init: {}",
            output
        );
        assert!(
            output.contains("Funzzy hello world"),
            "hello job output missing: {output}"
        );

        // Change example: a generic matching file triggers the `ls -a` job.
        write_to_file!(dir.join("notes.txt"));

        wait_until!(
            {
                output.clear();
                let mut log = std::fs::File::open(&log_path).expect("open log");
                log.read_to_string(&mut output).expect("read log");
                output.contains("notes.txt")
            },
            "generated change job must run on a matching file: {}",
            output
        );
    });
    let _ = std::fs::remove_dir_all(&dir);
}

/// Criterion 5: each documented enum/default/example is accepted by the
/// production parser; invalid alternatives still fail deterministically.
#[test]
fn documented_values_parse_and_invalid_alternatives_fail() {
    let dir = empty_dir("init-values");

    let valid = [
        // All on-section enum/default examples from the template.
        "on:\n  change: '**/*'\n  watch_backend: poll\n  poll_interval: 200ms\n  debounce: 500ms\n  respect_gitignore: true\n  output: quiet\n  success: echo ok > .fzz-success\n  failure: echo failed > .fzz-failed\njobs:\n  - name: a\n    run: echo a\n",
        // Job-side examples.
        "jobs:\n  - name: a\n    run: [\"echo\", \"{{filepath}}\"]\n    change: [\"**/*.rs\", \"**/*.md\"]\n    ignore: [\"**/*.log\"]\n    parallel: checks\n    cwd: scripts\n    env:\n      FOO: bar\n    service: true\n    output: show-on-failure\n",
        // Required defaults accept the documented scalar forms.
        "on:\n  change: '**/*'\n  concurrency: 2\njobs:\n  - name: a\n    run: echo a\n    run_on_init: true\n",
    ];
    for (i, config) in valid.iter().enumerate() {
        let out = check_ok(&dir, config);
        assert!(
            out.status.success(),
            "valid config #{i} must pass fzz check: {}",
            String::from_utf8_lossy(&out.stderr)
        );
    }

    let invalid = [
        // On-level and job-level values validated at parse time (fzz check).
        (
            "on:\n  change: '**/*'\n  output: loud\njobs:\n  - name: a\n    run: echo a\n",
            "loud",
        ),
        (
            "jobs:\n  - name: a\n    run: echo a\n    service: yes\n",
            "service",
        ),
        (
            "jobs:\n  - name: a\n    run: echo a\n    output: noisy\n",
            "noisy",
        ),
    ];
    for (config, needle) in invalid {
        let out = check_ok(&dir, config);
        assert!(!out.status.success(), "invalid config must fail: {config}");
        let message = format!(
            "{}{}",
            String::from_utf8_lossy(&out.stdout),
            String::from_utf8_lossy(&out.stderr)
        );
        assert!(
            message.contains(needle),
            "failure must name the offending value '{needle}': {message}"
        );
    }

    // Watch-only startup validation (mirrors the debounce gate): an invalid
    // `on.watch_backend` fails fast before the watch loop starts.
    let probe = dir.join("backend.yml");
    std::fs::write(
        &probe,
        "on:\n  change: '**/*'\n  watch_backend: bogus\njobs:\n  - name: a\n    run: echo a\n",
    )
    .expect("write backend probe");
    let watch = fzz()
        .arg("-c")
        .arg(&probe)
        .arg("watch")
        .current_dir(&dir)
        .output()
        .expect("failed to run fzz watch");
    assert!(
        !watch.status.success(),
        "invalid watch_backend must fail at watch startup"
    );
    let watch_out = String::from_utf8_lossy(&watch.stdout);
    assert!(
        watch_out.contains("bogus"),
        "watch failure must name the offending value: {watch_out}"
    );

    let _ = std::fs::remove_dir_all(&dir);
}

/// Criterion 4: template comments cover every canonical optional property and
/// advertise no unsupported property (parity with the option catalog).
#[test]
fn template_comments_cover_catalog_without_unsupported_properties() {
    use funzzy::option_catalog::{self, Owner};

    let dir = empty_dir("init-parity");
    let init = init_in(&dir);
    assert!(init.status.success(), "init failed");
    let content = std::fs::read_to_string(dir.join(".watch.yaml")).expect("read template");

    let comment_keys: Vec<String> = content
        .lines()
        .map(str::trim_start)
        .filter(|l| l.starts_with('#'))
        .filter_map(|l| {
            let rest = l.trim_start_matches('#').trim();
            let (key, _) = rest.split_once(':')?;
            // Property keys are single lowercase words (underscores allowed);
            // prose lines like "Next commands:" or "Comprehensive commented
            // starter:" never match.
            let key = key.trim();
            (key.len() > 0 && key.chars().all(|c| c.is_ascii_lowercase() || c == '_'))
                .then(|| key.to_string())
        })
        .collect();

    let known: Vec<&str> = option_catalog::property_names(Owner::On)
        .into_iter()
        .chain(option_catalog::property_names(Owner::Job))
        .chain(option_catalog::property_names(Owner::Root))
        .collect();

    // Every optional catalog property appears commented.
    for spec in option_catalog::on_specs()
        .iter()
        .chain(option_catalog::job_specs())
    {
        if !option_catalog::is_optional(spec) {
            continue;
        }
        assert!(
            comment_keys.iter().any(|k| k == spec.name),
            "catalog property '{}' missing from init comments",
            spec.name
        );
    }

    // No comment advertises an unsupported property (context words such as
    // "default:" and "values:" are excluded by shape — we only inspect keys).
    for key in &comment_keys {
        assert!(
            known.contains(&key.as_str()),
            "init template advertises unsupported property '{key}'"
        );
    }

    let _ = std::fs::remove_dir_all(&dir);
}

/// Criterion 8: `fzz config example minimal|parallel|agent` stay runnable and
/// do not inherit the human-commented init output accidentally.
#[test]
fn example_profiles_stay_lean_and_do_not_inherit_init_output() {
    for profile in ["minimal", "parallel", "agent"] {
        let out = fzz()
            .args(["config", "example", profile])
            .output()
            .expect("failed to run fzz config example");
        assert!(
            out.status.success(),
            "example {profile} must print: {}",
            String::from_utf8_lossy(&out.stderr)
        );
        let stdout = String::from_utf8_lossy(&out.stdout);
        assert!(
            !stdout.contains("Comprehensive commented starter"),
            "example {profile} must not inherit init header comments"
        );
        assert!(
            !stdout.contains("option documented in comments"),
            "example {profile} must not inherit init header text"
        );
    }
}
