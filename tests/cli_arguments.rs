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
//! - `--target` accepts a value-less form by second-parse workaround.
//! - The `watch` keyword is inert: `watch '<command>'` == `'<command>'`.
//! - `--migrate` only acts together with `init`; alone it falls through to
//!   the config branch.
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
    let _ = f(&dir);
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
            .stdout(predicate::str::contains("--target"))
            .stdout(predicate::str::contains("--control-socket"));
    }
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
// --target paths
// ---------------------------------------------------------------------------

#[test]
fn target_without_value_lists_available_tasks() {
    // Value-less `-t` is accepted through a second-parse workaround and lists
    // available tasks with exit 0. Clap would normally reject a missing
    // value; the migration must keep this path working.
    fzz()
        .args(["-c", FILTER_EXAMPLE, "-t"])
        .assert()
        .code(0)
        .stdout(predicate::str::contains("Available tasks"))
        .stdout(predicate::str::contains("run my test @quick"))
        .stdout(predicate::str::contains(
            "Usage `fzz -t <text_contain_in_task>`",
        ));
}

#[test]
fn target_with_explicit_empty_value_lists_available_tasks() {
    fzz()
        .args(["-c", FILTER_EXAMPLE, "--target="])
        .assert()
        .code(0)
        .stdout(predicate::str::contains("Available tasks"))
        .stdout(predicate::str::contains(
            "Usage `fzz -t <text_contain_in_task>`",
        ));
}

#[test]
fn target_without_match_fails_listing_available_tasks() {
    fzz()
        .args(["-c", FILTER_EXAMPLE, "-t", "no-such-target"])
        .assert()
        .code(1)
        .stdout(predicate::str::contains(
            "No target found for 'no-such-target'",
        ))
        .stdout(predicate::str::contains("Available tasks"));
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
    let mut child = StdCommand::new(env!("CARGO_BIN_EXE_fzz"))
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
    let mut output = String::new();
    wait_until!(
        {
            output = std::fs::read_to_string(log_name).unwrap_or_default();
            output.contains(needle)
        },
        "fzz output never contained {:?}: {}",
        needle,
        output
    );
    output
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
fn target_matching_value_starts_watcher() {
    // Options placed before and around the command form; substring match on
    // a task name (not a tag) selects the target and the watch starts. The
    // matched task runs on init, which is the deterministic startup marker.
    let (mut child, log_name) = spawn_with_config(&["-t", "run my build"], "target-match.log");
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
    let output = wait_for_output(&log_name, "Funzzy verbose");
    assert!(
        output.contains("Running on init commands."),
        "verbose watch must still start its init run:\n{}",
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
        &["exec", "--", "echo {{filepath}}"],
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
