#[path = "./macros.rs"]
mod macros;

#[allow(unused_imports)]
use std::{
    collections::HashMap,
    env,
    fs::File,
    process::{Command, Stdio},
    thread::sleep,
    time::Duration,
};

use crate::defer;
// use crate::shell;

#[allow(dead_code)]
pub struct Options {
    pub output_file: &'static str,
    pub example_file: &'static str,
}

static IS_RUNNING_MULTITHREAD: std::sync::Mutex<u8> = std::sync::Mutex::new(0);

/// Serializes a closure against every filesystem-touching integration test
/// (the same mutex `with_config`/`with_output`/`with_example` hold). Tests
/// that spawn their own watcher in a scratch dir must call this so they do
/// not run concurrently with harness tests and starve their wait budgets.
#[allow(dead_code)]
pub fn serialized<F: FnOnce()>(handler: F) {
    let mut is_running = IS_RUNNING_MULTITHREAD
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner());
    if *is_running == 1 {
        *is_running = 0;
    }
    loop {
        if *is_running == 0 {
            *is_running = 1;
            break;
        }
        sleep(Duration::from_millis(200));
    }
    defer!({
        *is_running = 0;
    });
    handler();
}

/// Per-run fixture root: `temp_dir()/funzzy-fixture-<pid>-<label>/` with a
/// private copy of the `examples/` tree.
///
/// Tests write trigger files and example configs watch `examples/workdir/**`
/// relative to the fzz working directory, so concurrent integration runs
/// (watcher generation, CI, manual invocations) would otherwise write the
/// same files and see each other's triggers. Each run instead gets its own
/// fixture and fzz is spawned with `current_dir = fixture`, which keeps the
/// relative-path glob behavior (`examples/workdir/**`) intact while making
/// every write and every watch root run-private.
#[allow(dead_code)]
pub fn fixture_root(label: &str) -> std::path::PathBuf {
    let root =
        std::env::temp_dir().join(format!("funzzy-fixture-{}-{}", std::process::id(), label));
    let _ = std::fs::remove_dir_all(&root);
    // Copy the trees example configs glob against. `src/**`/`tests/**` must
    // resolve inside the fixture or fzz warns "unknown file/directory".
    for tree in ["examples", "src", "tests"] {
        copy_dir_recursive(tree, &root.join(tree))
            .unwrap_or_else(|_| panic!("failed to copy {} tree into fixture", tree));
    }
    // Resolve symlink prefixes (macOS maps /var -> /private/var) so the
    // fixture paths the test writes, the paths notify reports, and the
    // `$PWD` template expansion are the same canonical strings.
    std::fs::canonicalize(&root).expect("failed to canonicalize fixture root")
}

/// Recursively copy a directory tree (follows symlinks, copying target
/// content). Used to build per-run fixtures from the checked-in `examples/`.
#[allow(dead_code)]
fn copy_dir_recursive(src: &str, dst: &std::path::Path) -> std::io::Result<()> {
    std::fs::create_dir_all(dst)?;
    for entry in std::fs::read_dir(src)? {
        let entry = entry?;
        let from = entry.path();
        let to = dst.join(entry.file_name());
        if from.is_dir() {
            copy_dir_recursive(&from.to_string_lossy(), &to)?;
        } else {
            std::fs::copy(&from, &to)?;
        }
    }
    Ok(())
}

#[allow(dead_code)]
pub const CLEAR_SCREEN: &str = "[2J";

#[cfg(not(feature = "test-integration"))]
#[allow(dead_code)]
pub fn with_example<F>(_: Options, _: F)
where
    F: FnOnce(&mut Command, File, &std::path::Path),
{
    println!("WARNING: Skipping integration tests");
}

#[cfg(not(feature = "test-integration"))]
#[allow(dead_code)]
pub fn with_config<F>(_: &std::path::Path, _: &str, _: F)
where
    F: FnOnce(&mut Command, File, &std::path::Path),
{
    println!("WARNING: Skipping integration tests");
}

#[cfg(not(feature = "test-integration"))]
#[allow(dead_code)]
pub fn with_output<F>(_output_file_path: &str, _handler: F)
where
    F: FnOnce(&mut Command, File, &std::path::Path),
{
    println!("WARNING: Skipping integration tests");
}

#[cfg(feature = "test-integration")]
#[allow(dead_code)]
pub fn with_example<F>(opts: Options, handler: F) -> ()
where
    F: FnOnce(&mut Command, File, &std::path::Path) -> (),
{
    let config_path = std::path::Path::new(opts.example_file);
    with_config(config_path, opts.output_file, handler)
}

#[cfg(feature = "test-integration")]
#[allow(dead_code)]
pub fn with_config<F>(config_path: &std::path::Path, output_file: &str, handler: F) -> ()
where
    F: FnOnce(&mut Command, File, &std::path::Path) -> (),
{
    let dir = env::current_dir().expect("error getting current directory");

    // Per-run fixture root so concurrent integration runs never share
    // trigger files or watch roots. fzz spawns with `current_dir` inside
    // the fixture, keeping relative config paths and relative globs
    // (`examples/workdir/**`) resolving against the fixture.
    let fixture = fixture_root(output_file);

    // Per-process log name so concurrent integration runs (watcher
    // generation, CI, manual) never create or truncate each other's logs.
    let log_name = format!("{}-{}", output_file, std::process::id());
    let _ = std::fs::remove_file(dir.join(&log_name));

    // NOTE: Execute ls command for debug purposes
    // very usefil to debug the tests that are failing
    // when building with nix: `nix build .#funzzy --verbose -L`
    // shell!("ls -la");

    // NOTE: OK, this is a bit hacky, but it's a simple way to avoid running
    // the tests from tests/*.rs in parallel.
    //
    // I'm aware of `cargo test -- --test-threads=1` option, but I want to run
    // all tests with `cargo test` in parallel and limit the parallelism only
    // for tests that write to the file system, like the integration tests.
    let mut is_running = IS_RUNNING_MULTITHREAD
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner());
    println!(
        "SINGLE THREAD: Is there another test running: {}",
        *is_running != 0
    );
    // If recovering from poisoned mutex, reset since test panicked
    if *is_running == 1 {
        *is_running = 0;
    }
    loop {
        // This here isn't really necessary, I noticed that since there is a
        // mutex lock, the test will run in sequence, but I'm leaving it here
        if *is_running == 0 {
            *is_running = 1;
            break;
        }

        let next_tick = 200;
        println!(
            "test already running, wait for the next tick in {} ms",
            next_tick
        );
        sleep(Duration::from_millis(next_tick));
    }
    defer!({
        *is_running = 0;
        let _ = std::fs::remove_dir_all(&fixture);
    });

    // check if the file exists if so fail
    assert!(
        !std::path::Path::new(&dir.join(&log_name)).exists(),
       "the log file already exists, make sure to give an unique log file to avoid multiple writes to same file: {}",
       dir.join(&log_name).display()
    );

    let bin_path = env!("CARGO_BIN_EXE_fzz");
    println!("Integration Tests: fzz bin from {}", bin_path);
    let output_log = File::create(dir.join(&log_name)).expect("error log file");

    let mut cmd = Command::new(bin_path);
    cmd.current_dir(&fixture);
    cmd.arg("-c");
    cmd.arg(config_path);
    if std::env::var("_TEST_FUNZZY_COLORED").is_err() {
        cmd.env("_TEST_FUNZZY_COLORED", "0");
    }
    if std::env::var("_TEST_FUNZZY_BAIL").is_err() {
        cmd.env("_TEST_FUNZZY_BAIL", "0");
    }
    if std::env::var("_TEST_FUNZZY_NON_BLOCK").is_err() {
        cmd.env("_TEST_FUNZZY_NON_BLOCK", "0");
    }
    cmd.stdout(Stdio::from(output_log));

    handler(
        &mut cmd,
        File::open(dir.join(&log_name)).expect("failed to open file"),
        &fixture,
    );

    // Idempotent cleanup: the log may already be gone under concurrent test
    // binaries (each process has its own mutex, so cross-binary parallelism
    // is not serialized); a missing file is not a failure.
    let _ = std::fs::remove_file(dir.join(&log_name));
}

#[cfg(feature = "test-integration")]
#[allow(dead_code)]
pub fn with_output<F>(output_file_path: &str, handler: F) -> ()
where
    F: FnOnce(&mut Command, File, &std::path::Path) -> (),
{
    let dir = env::current_dir().expect("error getting current directory");

    // Per-run fixture root so concurrent integration runs never share
    // trigger files or watch roots.
    let fixture = fixture_root(output_file_path);

    // Per-process log name so concurrent integration runs (watcher
    // generation, CI, manual) never create or truncate each other's logs.
    let log_name = format!("{}-{}", output_file_path, std::process::id());
    let _ = std::fs::remove_file(dir.join(&log_name));

    // NOTE: OK, this is a bit hacky, but it's a simple way to avoid running
    // the tests from tests/*.rs in parallel.
    //
    // I'm aware of `cargo test -- --test-threads=1` option, but I want to run
    // all tests with `cargo test` in parallel and limit the parallelism only
    // for tests that write to the file system, like the integration tests.
    let mut is_running = IS_RUNNING_MULTITHREAD
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner());
    println!(
        "SINGLE THREAD: Is there another test running: {}",
        *is_running != 0
    );
    // If recovering from poisoned mutex, reset since test panicked
    if *is_running == 1 {
        *is_running = 0;
    }
    loop {
        // This here isn't really necessary, I noticed that since there is a
        // mutex lock, the test will run in sequence, but I'm leaving it here
        if *is_running == 0 {
            *is_running = 1;
            break;
        }

        let next_tick = 200;
        println!(
            "test already running, wait for the next tick in {} ms",
            next_tick
        );
        sleep(Duration::from_millis(next_tick));
    }
    defer!({
        *is_running = 0;
        let _ = std::fs::remove_dir_all(&fixture);
    });

    // NOTE: Execute ls command for debug purposes
    // very usefil to debug the tests that are failing
    // when building with nix: `nix build .#funzzy --verbose -L`
    // shell!("ls -la");

    // check if the file exists if so fail
    assert!(
        !std::path::Path::new(&dir.join(&log_name)).exists(),
       "the log file already exists, make sure to give an unique log file to avoid multiple writes to same file: {}",
       dir.join(&log_name).display()
    );

    let bin_path = env!("CARGO_BIN_EXE_fzz");
    println!("Integration Tests: fzz bin from {}", bin_path);
    let output_file = File::create(dir.join(&log_name)).expect("error log file");

    let mut cmd = Command::new(bin_path);
    cmd.current_dir(&fixture);
    if std::env::var("_TEST_FUNZZY_COLORED").is_err() {
        cmd.env("_TEST_FUNZZY_COLORED", "0");
    }
    if std::env::var("_TEST_FUNZZY_BAIL").is_err() {
        cmd.env("_TEST_FUNZZY_BAIL", "0");
    }
    if std::env::var("_TEST_FUNZZY_NON_BLOCK").is_err() {
        cmd.env("_TEST_FUNZZY_NON_BLOCK", "0");
    }
    cmd.stdout(Stdio::from(output_file));

    handler(
        &mut cmd,
        File::open(dir.join(&log_name)).expect("failed to open file"),
        &fixture,
    );

    // Idempotent cleanup: the log may already be gone under concurrent test
    // binaries (each process has its own mutex, so cross-binary parallelism
    // is not serialized); a missing file is not a failure.
    let _ = std::fs::remove_file(dir.join(&log_name));
}

#[allow(dead_code)]
pub fn nonparallel<F>(handler: F) -> Result<(), Box<dyn std::error::Error>>
where
    F: FnOnce() -> Result<(), Box<dyn std::error::Error>>,
{
    // NOTE: OK, this is a bit hacky, but it's a simple way to avoid running
    // the tests from tests/*.rs in parallel.
    //
    // I'm aware of `cargo test -- --test-threads=1` option, but I want to run
    // all tests with `cargo test` in parallel and limit the parallelism only
    // for tests that write to the file system, like the integration tests.
    let mut is_running = IS_RUNNING_MULTITHREAD
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner());
    // If recovering from poisoned mutex, reset since test panicked
    if *is_running == 1 {
        *is_running = 0;
    }
    defer!({
        *is_running = 0;
    });

    handler()
}

/// Remove ANSI SGR color sequences (\x1b[...m) from output.
///
/// Assertions must be robust to the spawned binary being built with or
/// without the `test-integration` feature: outside the feature build,
/// `environment::is_enabled` reads the real `FUNZZY_COLORED`, so a direnv
/// export of `FUNZZY_COLORED=1` makes child output colored. Only SGR color
/// codes are stripped: the clear-screen sequence `\x1b[2J` is a semantic
/// marker some snapshots assert and is left intact.
#[allow(dead_code)]
pub fn strip_ansi_codes(input: &str) -> String {
    let mut result = String::with_capacity(input.len());
    let mut chars = input.chars().peekable();
    while let Some(c) = chars.next() {
        if c == '\u{1b}' && chars.peek() == Some(&'[') {
            chars.next(); // consume '['
                          // Peek ahead from the byte after '[': only strip CSI sequences
                          // that end in 'm' (SGR color codes). Other sequences (e.g. the
                          // clear-screen `\x1b[2J` some snapshots assert) are left intact.
            let mut lookahead = chars.clone();
            let mut is_sgr = false;
            let mut tail = String::new();
            for next in lookahead.by_ref() {
                if next == 'm' {
                    is_sgr = true;
                    break;
                }
                if !('\u{20}'..='\u{3f}').contains(&next) {
                    break;
                }
                tail.push(next);
            }
            if is_sgr {
                // consume the parameter bytes plus the 'm' we peeked
                for _ in tail.chars() {
                    chars.next();
                }
                chars.next();
            } else {
                result.push(c);
                result.push('[');
            }
        } else {
            result.push(c);
        }
    }
    result
}

#[allow(dead_code)]
pub fn clean_output(output_file: &str) -> String {
    output_file
        .lines()
        .map(|line| {
            // This line prints the time so is not deterministic
            if line.contains("Funzzy: finished in") {
                return "Funzzy: finished in 0.0s".to_string();
            }

            if line.contains("Duration: ") {
                if let Some(idx) = line.find("Duration: ") {
                    return format!("{}Duration: 0.0000s", &line[..idx]);
                }
            }

            line.to_string()
        })
        .filter(|line| !line.contains("@@@@"))
        .collect::<Vec<String>>()
        .join("\n")
}

#[allow(dead_code)]
pub fn with_env<F>(
    envvars: HashMap<String, String>,
    handler: F,
) -> Result<(), Box<dyn std::error::Error>>
where
    F: FnOnce() -> Result<(), Box<dyn std::error::Error>>,
{
    for key in envvars.keys() {
        let value = envvars.get(key).unwrap_or(&"".to_string()).clone();
        env::set_var(format!("_TEST_{}", key), value);
    }

    defer!({
        for key in envvars.keys() {
            env::remove_var(format!("_TEST_{}", key));
        }
    });

    handler()
}
