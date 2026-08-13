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

#[allow(dead_code)]
pub const CLEAR_SCREEN: &str = "[2J";

#[cfg(not(feature = "test-integration"))]
#[allow(dead_code)]
pub fn with_example<F>(_: Options, _: F) -> ()
where
    F: FnOnce(&mut Command, File) -> (),
{
    println!("WARNING: Skipping integration tests");
    ()
}

#[cfg(not(feature = "test-integration"))]
#[allow(dead_code)]
pub fn with_config<F>(_: &std::path::Path, _: &str, _: F) -> ()
where
    F: FnOnce(&mut Command, File) -> (),
{
    println!("WARNING: Skipping integration tests");
    ()
}

#[cfg(not(feature = "test-integration"))]
#[allow(dead_code)]
pub fn with_output<F>(output_file_path: &str, handler: F) -> ()
where
    F: FnOnce(&mut Command, File) -> (),
{
    println!("WARNING: Skipping integration tests");
    ()
}

#[cfg(feature = "test-integration")]
#[allow(dead_code)]
pub fn with_example<F>(opts: Options, handler: F) -> ()
where
    F: FnOnce(&mut Command, File) -> (),
{
    let config_path = std::path::Path::new(opts.example_file);
    with_config(config_path, opts.output_file, handler)
}

#[cfg(feature = "test-integration")]
#[allow(dead_code)]
pub fn with_config<F>(config_path: &std::path::Path, output_file: &str, handler: F) -> ()
where
    F: FnOnce(&mut Command, File) -> (),
{
    let dir = env::current_dir().expect("error getting current directory");

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
    );

    std::fs::remove_file(dir.join(&log_name)).expect("failed to remove file after running test");
}

#[cfg(feature = "test-integration")]
#[allow(dead_code)]
pub fn with_output<F>(output_file_path: &str, handler: F) -> ()
where
    F: FnOnce(&mut Command, File) -> (),
{
    let dir = env::current_dir().expect("error getting current directory");

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
    );

    std::fs::remove_file(dir.join(&log_name)).expect("failed to remove file after running test");
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
