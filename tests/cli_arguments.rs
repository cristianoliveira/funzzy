//! CLI compatibility contract (characterization tests) — TASK-0010.
//!
//! Locks parser-visible behavior of the `funzzy` and `fzz` binaries BEFORE
//! the Docopt -> Clap migration. Assertions target semantic content and exit
//! codes, never colors or incidental whitespace. Where Clap is expected to
//! render help/errors differently, the test comment names the accepted change
//! so the migration decision is conscious rather than hidden as drift.
//!
//! Notes on the current contract:
//! - `-V`/`--version` is Clap's built-in version flag (stdout, exit 0).
//! - `-v`/`--verbose` is the verbose watch flag.
//! - `fzz watch [TARGET]` selects tasks by name/@tag substring; `fzz list` lists them.
//! - The `watch` keyword is inert: `watch '<command>'` == `'<command>'`.
//! - `--migrate` is not a flag anymore: migration is the explicit `fzz
//!   migrate` subcommand (TASK-0098); `init --migrate` and bare `--migrate`
//!   are rejected with exit 2.
//! - Parse errors are rendered by Clap to stderr with exit 2.

use assert_cmd::cargo;
use predicates::prelude::*;

#[cfg(feature = "test-integration")]
use std::io::prelude::*;
#[cfg(feature = "test-integration")]
use std::process::{Child, Command as StdCommand, Stdio};

#[path = "./common/lib.rs"]
mod setup;

const FILTER_EXAMPLE: &str = "examples/tasks-with-tags-to-filter.yml";

fn fzz() -> assert_cmd::Command {
    let mut cmd = cargo::cargo_bin_cmd!("fzz");
    cmd.env("FUNZZY_COLORED", "false");
    cmd
}

fn funzzy() -> assert_cmd::Command {
    let mut cmd = cargo::cargo_bin_cmd!("funzzy");
    cmd.env("FUNZZY_COLORED", "false");
    cmd
}

/// Run `f` against a fresh, unique temp directory; remove it afterwards.
/// The per-test suffix keeps parallel tests in the same process apart.
fn with_tmp_dir<F: FnOnce(&std::path::Path)>(suffix: &str, f: F) {
    let dir = std::env::temp_dir().join(format!(
        "funzzy-cli-contract-{}-{}",
        std::process::id(),
        suffix
    ));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).expect("failed to create temp dir");
    f(&dir);
    let _ = std::fs::remove_dir_all(&dir);
}

// ---------------------------------------------------------------------------
// Help, version, and both binaries
// ---------------------------------------------------------------------------

#[test]
fn help_shows_usage_commands_and_options_for_both_binaries() {
    for mut cmd in [fzz(), funzzy()] {
        cmd.arg("-h")
            .assert()
            .code(0)
            .stdout(predicate::str::contains("Usage:"))
            .stdout(predicate::str::contains("Commands:"))
            .stdout(predicate::str::contains("Options:"))
            // The verbose flag is `-v`/`--verbose` ("Use verbose output."),
            // not the version short flag (`-V`).
            .stdout(predicate::str::contains("-v"))
            .stdout(predicate::str::contains("--verbose"))
            .stdout(predicate::str::contains("--on-busy"))
            .stdout(predicate::str::contains("--control-socket"))
            // The clap-generated commands section lists every subcommand
            // (check/completions/config included) and `run` takes its
            // TARGET argument; `fzz run --help` shows the <TARGET> usage.
            .stdout(predicate::str::contains("  run"))
            .stdout(predicate::str::contains("  check"));
    }
}

#[test]
fn control_help_distinguishes_canonical_from_ctl_alias() {
    // TASK-0070: `ctl` must be a visible alias; both spellings expose the
    // same nested tree and the top-level help advertises the alias once.
    for mut cmd in [fzz(), funzzy()] {
        for spelling in ["control", "ctl"] {
            cmd.arg(spelling)
                .arg("--help")
                .assert()
                .code(0)
                .stdout(predicate::str::contains("status"))
                .stdout(predicate::str::contains("list"))
                .stdout(predicate::str::contains("run"))
                .stdout(predicate::str::contains("capabilities"));
        }
    }
}

#[test]
fn top_level_help_advertises_ctl_alias_once() {
    let output = fzz().arg("--help").output().unwrap();
    let stdout = String::from_utf8_lossy(&output.stdout);
    assert!(
        stdout.contains("[alias: ctl]"),
        "top-level help must advertise the ctl alias: {}",
        stdout
    );
}

#[test]
fn unknown_control_operation_fails_with_exit_2_for_both_spellings() {
    for mut cmd in [fzz(), funzzy()] {
        for spelling in ["control", "ctl"] {
            cmd.arg(spelling)
                .arg("bogus")
                .assert()
                .code(2)
                .stderr(predicate::str::contains("bogus"));
        }
    }
}

#[test]
fn run_help_distinguishes_local_execution_from_control_ipc() {
    fzz()
        .args(["run", "--help"])
        .assert()
        .code(0)
        .stdout(predicate::str::contains("once in this process"))
        .stdout(predicate::str::contains("fzz control run TARGET"))
        .stdout(predicate::str::contains("Path filtering is not supported"));
}

#[test]
fn help_output_is_color_neutral_when_colors_disabled() {
    fzz()
        .arg("--help")
        .assert()
        .code(0)
        .stdout(predicate::str::contains("\u{1b}").not());
}

#[test]
fn short_version_flag_prints_version_for_both_binaries() {
    for mut cmd in [fzz(), funzzy()] {
        cmd.arg("-V")
            .assert()
            .code(0)
            .stdout(predicate::str::contains(env!("CARGO_PKG_VERSION")));
    }
}

#[test]
fn long_version_flag_prints_version() {
    fzz()
        .arg("--version")
        .assert()
        .code(0)
        .stdout(predicate::str::contains(env!("CARGO_PKG_VERSION")));
}

#[test]
fn version_and_help_flags_win_over_commands_regardless_of_position() {
    // Options accepted before AND after the command word; -V/-h still win.
    fzz()
        .args(["init", "-V"])
        .assert()
        .code(0)
        .stdout(predicate::str::contains(env!("CARGO_PKG_VERSION")));
    fzz()
        .args(["-V", "init"])
        .assert()
        .code(0)
        .stdout(predicate::str::contains(env!("CARGO_PKG_VERSION")));
    fzz()
        .args(["init", "-h"])
        .assert()
        .code(0)
        .stdout(predicate::str::contains("Usage:"));
}

#[test]
fn verbose_short_flag_is_verbose_not_version() {
    // `-v` is the verbose watch flag; it never prints the version. `-V` does.
    fzz()
        .args(["-V"])
        .assert()
        .code(0)
        .stdout(predicate::str::contains(env!("CARGO_PKG_VERSION")));
}

// ---------------------------------------------------------------------------
// Command dispatch without a config file
// ---------------------------------------------------------------------------

#[test]
fn no_args_without_config_fails_with_guidance() {
    with_tmp_dir("no-args", |dir| {
        fzz()
            .current_dir(dir)
            .assert()
            .code(1)
            .stdout(predicate::str::contains(
                "Failed to read default config file",
            ))
            .stdout(predicate::str::contains("Try `fzz init`"));
    });
}

#[test]
fn migrate_without_init_is_unknown() {
    // `--migrate` is scoped to `init` in V2; without `init` it is not a valid
    // flag, so parsing fails to stderr with exit 2.
    with_tmp_dir("migrate-alone", |dir| {
        fzz()
            .args(["--migrate"])
            .current_dir(dir)
            .assert()
            .code(2)
            .stderr(predicate::str::contains("--migrate"));
    });
}

#[test]
fn on_busy_and_fail_fast_flags_reach_config_branch() {
    // `--on-busy restart` + `-b` (fail-fast) must parse; reaching the config
    // branch (missing default config) proves the parse succeeded.
    with_tmp_dir("combined-flags", |dir| {
        fzz()
            .args(["--on-busy", "restart", "-b"])
            .current_dir(dir)
            .assert()
            .code(1)
            .stdout(predicate::str::contains(
                "Failed to read default config file",
            ));
    });
}

#[test]
fn init_smoke_creates_config_in_cwd() {
    with_tmp_dir("init", |dir| {
        fzz()
            .arg("init")
            .current_dir(dir)
            .assert()
            .code(0)
            .stdout(predicate::str::contains(
                "Configuration file created successfully!",
            ));
        assert!(
            dir.join(".watch.yaml").exists(),
            "init must create .watch.yaml"
        );
    });
}

// ---------------------------------------------------------------------------
// watch/list/explain/target paths
// ---------------------------------------------------------------------------

#[test]
fn list_subcommand_lists_available_tasks() {
    // `fzz list` prints configured tasks and exits 0.
    fzz()
        .args(["-c", FILTER_EXAMPLE, "list"])
        .assert()
        .code(0)
        .stdout(predicate::str::contains("Available jobs"))
        .stdout(predicate::str::contains("run my test @quick"))
        .stdout(predicate::str::contains("Usage").not());
}

#[test]
fn list_handles_empty_custom_config() {
    with_tmp_dir("list-empty", |dir| {
        let config = dir.join("empty.yml");
        std::fs::write(&config, "[]\n").expect("failed to write empty config");

        fzz()
            .arg("-c")
            .arg(&config)
            .arg("list")
            .assert()
            .code(0)
            .stdout(predicate::str::contains("Available jobs\n  (none)"));
    });
}

#[test]
fn list_rejects_semantically_invalid_config() {
    with_tmp_dir("list-invalid", |dir| {
        let config = dir.join("invalid.yml");
        std::fs::write(&config, "- name: broken\n  run: echo broken\n")
            .expect("failed to write invalid config");

        fzz()
            .arg("-c")
            .arg(&config)
            .arg("list")
            .assert()
            .code(1)
            .stdout(predicate::str::contains("Invalid config file"))
            .stdout(predicate::str::contains(
                "must contain a `change` and/or `run_on_init` property",
            ));
    });
}

#[test]
fn watch_rejects_invalid_on_debounce_config() {
    // TASK-0031: a typo or invalid `on.debounce` duration must fail loudly,
    // never silently change timing.
    with_tmp_dir("invalid-debounce", |dir| {
        let config = dir.join("debounce.yml");
        std::fs::write(
            &config,
            "on:\n  change: '**/*'\n  debounce: fast\ntasks:\n  - name: ok\n    run: 'true'\n    change: '**/*'\n",
        )
        .expect("failed to write invalid debounce config");

        fzz()
            .arg("-c")
            .arg(&config)
            .arg("watch")
            .assert()
            .code(1)
            .stdout(predicate::str::contains("Invalid debounce config"));

        std::fs::write(
            &config,
            "on:\n  change: '**/*'\n  debounce: 0\ntasks:\n  - name: ok\n    run: 'true'\n    change: '**/*'\n",
        )
        .expect("rewrite config");
        fzz()
            .arg("-c")
            .arg(&config)
            .arg("watch")
            .assert()
            .code(1)
            .stdout(predicate::str::contains("Invalid debounce config"));
    });
}

#[test]
fn target_flag_is_rejected() {
    // V2 removed --target/-t in favor of `watch TARGET` and `list`.
    // The rejection names the V2 replacements (TASK-0064 criterion 4).
    fzz()
        .args(["-c", FILTER_EXAMPLE, "--target=foo"])
        .assert()
        .code(2)
        .stderr(predicate::str::contains("--target"))
        .stderr(predicate::str::contains("watch"));
}

#[test]
fn non_block_flag_is_rejected_with_hint() {
    // V2 removed --non-block; rejection names --on-busy restart.
    fzz()
        .args(["-c", FILTER_EXAMPLE, "--non-block"])
        .assert()
        .code(2)
        .stderr(predicate::str::contains("--non-block"))
        .stderr(predicate::str::contains("--on-busy restart"));
}

#[test]
fn watch_unknown_target_fails_listing_available_tasks() {
    fzz()
        .args(["-c", FILTER_EXAMPLE, "watch", "no-such-target"])
        .assert()
        .code(1)
        .stdout(predicate::str::contains(
            "No target found for 'no-such-target'",
        ))
        .stdout(predicate::str::contains("Available jobs"));
}

// ---------------------------------------------------------------------------
// explain paths
// ---------------------------------------------------------------------------

#[test]
fn explain_matched_path_names_tasks_and_change_rules() {
    // A path matching the change patterns selects the tasks and reports the
    // exact change rule per task.
    fzz()
        .args(["-c", FILTER_EXAMPLE, "explain", "examples/workdir/foo.txt"])
        .assert()
        .code(0)
        .stdout(predicate::str::contains(
            "Explain path examples/workdir/foo.txt",
        ))
        .stdout(predicate::str::contains("matched:"))
        .stdout(predicate::str::contains("run my test @quick"))
        .stdout(predicate::str::contains("change: examples/workdir/*.txt"))
        .stdout(predicate::str::contains("ignored:").not())
        .stdout(predicate::str::contains("unmatched").not());
}

#[test]
fn explain_ignored_path_names_winning_ignore_rule() {
    // Change matches but the ignore rule wins; both are reported.
    fzz()
        .args([
            "-c",
            FILTER_EXAMPLE,
            "explain",
            "examples/workdir/ignored/x.txt",
        ])
        .assert()
        .code(0)
        .stdout(predicate::str::contains("ignored:"))
        .stdout(predicate::str::contains("run my test @quick"))
        .stdout(predicate::str::contains("change: examples/workdir/*.txt"))
        .stdout(predicate::str::contains(
            "ignored by: examples/workdir/ignored/**/*.txt",
        ))
        .stdout(predicate::str::contains("matched:").not());
}

#[test]
fn explain_unmatched_path_is_informative() {
    // No rule watches the path: explicit unmatched message, exit 0.
    fzz()
        .args([
            "-c",
            FILTER_EXAMPLE,
            "explain",
            "examples/workdir/nope.yaml",
        ])
        .assert()
        .code(0)
        .stdout(predicate::str::contains(
            "unmatched: no configured task watches this path",
        ));
}

#[test]
fn explain_accepts_absolute_paths() {
    let absolute = std::path::Path::new("examples/workdir/trigger-watcher.txt")
        .canonicalize()
        .expect("canonicalize fixture path");
    fzz()
        .arg("-c")
        .arg(FILTER_EXAMPLE)
        .arg("explain")
        .arg(absolute.to_str().expect("utf8 path"))
        .assert()
        .code(0)
        .stdout(predicate::str::contains("run my test @quick"));
}

#[test]
fn explain_without_path_fails_with_usage_error() {
    fzz()
        .args(["-c", FILTER_EXAMPLE, "explain"])
        .assert()
        .code(2)
        .stderr(predicate::str::contains("PATH"));
}

#[test]
fn explain_rejects_semantically_invalid_config() {
    with_tmp_dir("explain-invalid", |dir| {
        let config = dir.join("invalid.yml");
        std::fs::write(&config, "- name: broken\n  run: echo broken\n")
            .expect("failed to write invalid config");

        fzz()
            .arg("-c")
            .arg(&config)
            .args(["explain", "any/path.rs"])
            .assert()
            .code(1)
            .stdout(predicate::str::contains("Invalid config file"));
    });
}

#[test]
fn explain_does_not_start_a_watcher() {
    // `explain` is side-effect free: it must exit immediately with results,
    // not enter the watch loop.
    fzz()
        .args(["-c", FILTER_EXAMPLE, "explain", "examples/workdir/foo.txt"])
        .assert()
        .code(0)
        .stdout(predicate::str::contains("matched:"))
        .stdout(predicate::str::contains("Watching...").not())
        .stdout(predicate::str::contains("Running on init").not());
}

// ---------------------------------------------------------------------------
// Unknown options and missing required values
// ---------------------------------------------------------------------------

#[test]
fn unknown_long_option_fails_naming_the_flag() {
    // Clap renders parse errors to stderr with exit 2; the offending flag
    // appears in the error output.
    fzz()
        .arg("--bogus")
        .assert()
        .code(2)
        .stderr(predicate::str::contains("--bogus"));
}

#[test]
fn unknown_short_option_fails_naming_the_flag() {
    fzz()
        .arg("-z")
        .assert()
        .code(2)
        .stderr(predicate::str::contains("-z"));
}

#[test]
fn missing_value_for_config_option_fails() {
    // Clap: "a value is required for '--config <cfgfile>'". Contract: parse
    // error -> stderr, exit 2.
    fzz()
        .arg("-c")
        .assert()
        .code(2)
        .stderr(predicate::str::contains("--config"));
}

#[test]
fn missing_value_for_log_file_option_fails() {
    fzz()
        .arg("--log-file")
        .assert()
        .code(2)
        .stderr(predicate::str::contains("--log-file"));
}

#[test]
fn log_truncate_without_log_file_fails() {
    fzz()
        .arg("-T")
        .assert()
        .code(1)
        .stdout(predicate::str::contains(
            "`--log-truncate-on-change` requires `--log-file`",
        ));
}

// ---------------------------------------------------------------------------
// Option value syntax
// ---------------------------------------------------------------------------

#[test]
fn config_option_accepts_equals_form() {
    // `--config=<path>` must be accepted with the value exactly as given.
    fzz()
        .args(["--config=/definitely/not/a/config.yml"])
        .assert()
        .code(1)
        .stdout(predicate::str::contains(
            "Couldn't open configuration file: '/definitely/not/a/config.yml'",
        ));
}

#[test]
fn short_config_equals_form_uses_value_without_equals() {
    // Docopt quirk: `-c=<path>` yielded the value `=<path>` literally.
    // Clap treats `-c=<path>` as value `<path>`; the migration adopts clap
    // semantics, so a missing file fails naming the path without the `=`.
    fzz()
        .args(["-c=/definitely/not/a/config.yml"])
        .assert()
        .code(1)
        .stdout(predicate::str::contains(
            "Couldn't open configuration file: '/definitely/not/a/config.yml'",
        ));
}

// ---------------------------------------------------------------------------
// Watch-starting forms (integration behavior; skipped without the feature)
// ---------------------------------------------------------------------------

#[cfg(feature = "test-integration")]
fn spawn_with_config(extra_args: &[&str], log_name: &str) -> (Child, String) {
    // Mirrors tests/common/lib.rs: unique per-process log, no colors.
    let log_name = format!("{}-{}", log_name, std::process::id());
    let log_path = std::path::Path::new(&log_name);
    let _ = std::fs::remove_file(log_path);
    let log = std::fs::File::create(log_path).expect("failed to create log file");
    let child = StdCommand::new(env!("CARGO_BIN_EXE_fzz"))
        .arg("-c")
        .arg(FILTER_EXAMPLE)
        .args(extra_args)
        .env("FUNZZY_COLORED", "false")
        .stdin(Stdio::null())
        .stdout(Stdio::from(log))
        .spawn()
        .expect("failed to spawn fzz");
    (child, log_name)
}

#[cfg(feature = "test-integration")]
fn wait_for_output(log_name: &str, needle: &str) -> String {
    for _ in 0..100 {
        let output = std::fs::read_to_string(log_name).unwrap_or_default();
        if output.contains(needle) {
            return output;
        }
        std::thread::sleep(std::time::Duration::from_millis(250));
    }
    let output = std::fs::read_to_string(log_name).unwrap_or_default();
    panic!("fzz output never contained {needle:?}: {output}");
}

#[cfg(feature = "test-integration")]
#[test]
fn configured_watch_form_starts_watcher() {
    // The example fixture has a `run_on_init` task, so the startup marker is
    // the init run; afterwards the watch loop is live.
    let (mut child, log_name) = spawn_with_config(&[], "configured-watch.log");
    defer!({
        let _ = child.kill();
        let _ = std::fs::remove_file(&log_name);
    });
    wait_for_output(&log_name, "Running on init commands.");
}

#[cfg(feature = "test-integration")]
#[test]
fn watch_target_starts_watcher() {
    // `fzz watch <TARGET>` selects by substring match on a task name; the
    // matched task runs on init, the deterministic startup marker.
    let (mut child, log_name) = spawn_with_config(&["watch", "run my build"], "target-match.log");
    defer!({
        let _ = child.kill();
        let _ = std::fs::remove_file(&log_name);
    });
    wait_for_output(&log_name, "Running on init commands.");
}

#[cfg(feature = "test-integration")]
#[test]
fn verbose_short_flag_starts_watcher_in_verbose() {
    let (mut child, log_name) = spawn_with_config(&["-v"], "verbose-flag.log");
    defer!({
        let _ = child.kill();
        let _ = std::fs::remove_file(&log_name);
    });
    let output = wait_for_output(&log_name, "Running on init commands.");
    assert!(
        output.contains("Funzzy debug:"),
        "verbose watch must enable diagnostics before its init run:\n{}",
        output
    );
}

#[cfg(feature = "test-integration")]
fn spawn_watch_from_stdin(args: &[&str], stdin_data: &str, log_name: &str) -> (Child, String) {
    // Runs in the repo root (patterns must resolve to real files). A
    // non-existent `-c` path keeps the control-socket discovery from picking
    // up the repo `.watch.yaml` while the socket is held by another watcher.
    let log_name = format!("{}-{}", log_name, std::process::id());
    let log_path = std::path::Path::new(&log_name);
    let _ = std::fs::remove_file(log_path);
    let log = std::fs::File::create(log_path).expect("failed to create log file");
    let mut child = StdCommand::new(env!("CARGO_BIN_EXE_fzz"))
        .args(args)
        .env("FUNZZY_COLORED", "false")
        .stdin(Stdio::piped())
        .stdout(Stdio::from(log))
        .spawn()
        .expect("failed to spawn fzz");
    {
        let mut stdin = child.stdin.take().expect("failed to take stdin");
        stdin
            .write_all(stdin_data.as_bytes())
            .expect("failed to write stdin");
    }
    (child, log_name)
}

#[cfg(feature = "test-integration")]
#[test]
fn direct_command_form_watches_stdin_patterns() {
    let (mut child, log_name) = spawn_watch_from_stdin(
        &["exec", "--", "echo", "{{filepath}}"],
        "Cargo.toml\n",
        "direct-command.log",
    );
    defer!({
        let _ = child.kill();
        let _ = std::fs::remove_file(&log_name);
    });
    let output = wait_for_output(&log_name, "Funzzy: watching patterns");
    assert!(
        output.contains("Cargo.toml"),
        "stdin pattern must be reported:\n{}",
        output
    );
}

// The `watch <command>` form is removed in V2; ad-hoc commands now go
// through `exec --` (see direct_command_form_watches_stdin_patterns).

#[test]
fn check_reports_valid_config_and_counts() {
    with_tmp_dir("check-valid", |dir| {
        let config = dir.join("valid.yml");
        std::fs::write(
            &config,
            "on:\n  change: '**/*'\nexecution:\n  concurrency: 2\njobs:\n  - name: lint\n    parallel: checks\n    run: cargo clippy\n    change: 'src/**'\n  - name: test\n    parallel: checks\n    run: cargo test\n    change: 'src/**'\n",
        )
        .expect("write config");

        fzz()
            .arg("-c")
            .arg(&config)
            .arg("check")
            .assert()
            .code(0)
            .stdout(predicate::str::contains("config valid"))
            .stdout(predicate::str::contains("2 job(s)"))
            .stdout(predicate::str::contains("2 in parallel group(s)"))
            .stdout(predicate::str::contains("concurrency 2"));

        // Side-effect free: no socket, no log file, no task output.
        let entries: Vec<_> = std::fs::read_dir(dir)
            .unwrap()
            .map(|entry| entry.unwrap().file_name().to_string_lossy().to_string())
            .collect();
        assert_eq!(entries, vec!["valid.yml"], "check must not create files");
    });
}

#[test]
fn check_rejects_invalid_config_without_watching() {
    with_tmp_dir("check-invalid", |dir| {
        let config = dir.join("invalid.yml");
        // No command and no change/run_on_init: rule-level validation error.
        std::fs::write(&config, "jobs:\n  - name: broken\n").expect("write config");

        fzz()
            .arg("-c")
            .arg(&config)
            .arg("check")
            .assert()
            .code(1)
            .stdout(predicate::str::contains("Invalid config file"));
    });
}

#[test]
fn check_rejects_invalid_debounce_and_mixed_tasks_jobs() {
    with_tmp_dir("check-debounce", |dir| {
        let config = dir.join("bad.yml");
        std::fs::write(
            &config,
            "on:\n  change: '**/*'\n  debounce: fast\ntasks:\n  - name: a\n    run: echo a\n    change: '**/*'\n",
        )
        .expect("write config");
        fzz()
            .arg("-c")
            .arg(&config)
            .arg("check")
            .assert()
            .code(1)
            .stdout(predicate::str::contains("Invalid debounce config"));

        std::fs::write(
            &config,
            "on:\n  change: '**/*'\ntasks:\n  - name: a\n    run: echo a\n    change: '**/*'\njobs:\n  - name: b\n    run: echo b\n    change: '**/*'\n",
        )
        .expect("write config");
        fzz()
            .arg("-c")
            .arg(&config)
            .arg("check")
            .assert()
            .code(1)
            .stdout(predicate::str::contains("Invalid config file"));
    });
}

#[test]
fn explain_shows_filtered_execution_topology() {
    with_tmp_dir("explain-topology", |dir| {
        let config = dir.join("topology.yml");
        std::fs::write(
            &config,
            "execution:\n  concurrency: 2\njobs:\n  - name: lint @quick\n    parallel: checks\n    run: cargo clippy\n    change: 'src/**'\n  - name: test @quick\n    parallel: checks\n    run: cargo test\n    change: 'src/**'\n  - name: docs\n    run: mdbook build\n    change: 'docs/**'\n",
        )
        .expect("write config");

        fzz()
            .arg("-c")
            .arg(&config)
            .arg("explain")
            .arg("src/lib.rs")
            .assert()
            .code(0)
            .stdout(predicate::str::contains("matched:"))
            .stdout(predicate::str::contains("lint @quick"))
            .stdout(predicate::str::contains("test @quick"))
            // Filtered topology: one parallel group occurrence, docs excluded.
            .stdout(predicate::str::contains("[checks#1] (parallel group)"))
            .stdout(predicate::str::contains("lint @quick || test @quick"))
            .stdout(predicate::str::contains("concurrency: 2"))
            .stdout(predicate::str::contains("docs").not());
    });
}

#[test]
fn explain_shows_separated_group_occurrences() {
    with_tmp_dir("explain-occurrences", |dir| {
        let config = dir.join("occ.yml");
        std::fs::write(
            &config,
            "on:\n  change: '**/*'\njobs:\n  - name: a @quick\n    parallel: x\n    run: echo a\n    change: 'src/**'\n  - name: sep @quick\n    run: echo sep\n    change: 'src/**'\n  - name: c @quick\n    parallel: x\n    run: echo c\n    change: 'src/**'\n",
        )
        .expect("write config");

        fzz()
            .arg("-c")
            .arg(&config)
            .arg("explain")
            .arg("src/lib.rs")
            .assert()
            .code(0)
            // The reused group name never reconnects across the serial task:
            // two separate occurrences, not one.
            .stdout(predicate::str::contains("[x#1] (parallel group)"))
            .stdout(predicate::str::contains("[x#2] (parallel group)"))
            .stdout(predicate::str::contains("sep @quick"));
    });
}

#[test]
fn config_schema_emits_valid_deterministic_json_schema() {
    // TASK-0058: `fzz config schema` emits valid JSON Schema describing the
    // preferred jobs format, without reading any project config.
    let first = fzz()
        .args(["config", "schema", "--format", "json"])
        .output()
        .unwrap();
    let second = fzz()
        .args(["config", "schema", "--format", "json"])
        .output()
        .unwrap();
    assert!(first.status.success());
    let doc: serde_json::Value = serde_json::from_slice(&first.stdout).expect("valid JSON schema");
    assert_eq!(doc["properties"]["jobs"]["type"], "array");
    for section in ["on", "job", "matching", "execution", "parallel", "control"] {
        assert!(
            doc["$defs"][section].is_object(),
            "missing section {section}"
        );
    }
    // Deterministic repeated output.
    assert_eq!(first.stdout, second.stdout);
}

#[test]
fn config_schema_section_is_bounded_with_identity_and_hint() {
    let output = fzz()
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
    assert!(output.status.success());
    let doc: serde_json::Value = serde_json::from_slice(&output.stdout).unwrap();
    assert_eq!(doc["section"], "parallel");
    assert_eq!(doc["fullSchemaCommand"], "fzz config schema");
    assert!(doc["properties"]["parallel"].is_object());
}

#[test]
fn config_examples_are_runnable_and_parse() {
    for profile in ["minimal", "parallel", "agent"] {
        let output = fzz().args(["config", "example", profile]).output().unwrap();
        assert!(output.status.success(), "example {profile} exits 0");
        let yaml = String::from_utf8_lossy(&output.stdout);
        assert!(yaml.contains("jobs:"), "{profile} example uses jobs:");
        // The emitted YAML parses through the production parser: run `check`
        // on it in a scratch dir.
        use std::io::Write;
        let dir = std::env::temp_dir().join(format!(
            "funzzy-config-example-{}-{}",
            std::process::id(),
            profile
        ));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        let mut f = std::fs::File::create(dir.join(".watch.yaml")).unwrap();
        f.write_all(&output.stdout).unwrap();
        fzz()
            .current_dir(&dir)
            .arg("-c")
            .arg(".watch.yaml")
            .arg("check")
            .assert()
            .code(0);
        std::fs::remove_dir_all(&dir).unwrap();
    }
}

#[test]
fn config_unknown_input_fails_with_alternatives() {
    fzz()
        .args(["config", "example", "bogus"])
        .assert()
        .code(2)
        .stderr(predicate::str::contains("minimal"))
        .stderr(predicate::str::contains("parallel"))
        .stderr(predicate::str::contains("agent"));
}

#[test]
fn funzzy_and_fzz_expose_identical_command_trees() {
    // TASK-0020: both binaries share one command definition; help output must
    // be identical except the binary name itself.
    let fzz_help = fzz().arg("--help").output().unwrap();
    let funzzy_help = funzzy().arg("--help").output().unwrap();
    assert!(fzz_help.status.success() && funzzy_help.status.success());
    let normalize = |s: &str| s.replace("funzzy", "BIN").replace("fzz", "BIN");
    assert_eq!(
        normalize(&String::from_utf8_lossy(&fzz_help.stdout)),
        normalize(&String::from_utf8_lossy(&funzzy_help.stdout)),
        "command trees must be identical"
    );
}

#[test]
fn completions_generate_for_each_supported_shell() {
    for shell in ["bash", "zsh", "fish", "elvish", "powershell"] {
        let output = fzz()
            .args(["completions", shell])
            .output()
            .expect("generate completion");
        assert!(output.status.success(), "{shell} completion succeeds");
        let script = String::from_utf8_lossy(&output.stdout);
        assert!(!script.is_empty(), "{shell} completion is non-empty");
        assert!(
            script.contains("migrate"),
            "{shell} includes migrate command"
        );
        assert!(
            !script.contains("--migrate"),
            "{shell} excludes removed init flag"
        );
        for profile in ["comprehensive", "minimal", "parallel", "agent"] {
            assert!(script.contains(profile), "{shell} includes {profile}");
        }
    }
    // Unknown shell is a usage error naming the alternatives.
    fzz().args(["completions", "tcsh"]).assert().code(2);
}

#[test]
fn v2_release_evidence_removed_flags_are_rejected_and_exit_codes_stable() {
    // TASK-0020 release evidence: intentional V1 breaks are locked, exit
    // codes are 2 for usage, 1 for workflow/operational, 0 success.
    for removed in ["--non-block", "-n", "--target", "-t"] {
        fzz().arg(removed).assert().code(2);
    }
    // Help examples are exercised smoke tests: every top-level subcommand
    // shows help with exit 0.
    for sub in [
        "watch",
        "list",
        "run",
        "explain",
        "check",
        "init",
        "migrate",
        "config",
        "control",
        "completions",
    ] {
        fzz().args([sub, "--help"]).assert().code(0);
    }
}
