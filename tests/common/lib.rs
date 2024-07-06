#[path = "./macros.rs"]
mod macros;

use std::{
    env,
    fs::File,
    process::{Command, Stdio},
    thread::sleep,
    time::Duration,
};

use crate::defer;

#[allow(dead_code)]
pub struct Options {
    pub output_file: &'static str,
    pub example_file: &'static str,
}

static IS_RUNNING_MULTITHREAD: std::sync::Mutex<u8> = std::sync::Mutex::new(0);

#[allow(dead_code)]
pub const CLEAR_SCREEN: &str = "[2J";

#[allow(dead_code)]
pub fn with_example<F>(opts: Options, handler: F) -> ()
where
    F: FnOnce(&mut Command, File) -> (),
{
    let dir = env::current_dir().expect("error getting current directory");

    let _ = std::fs::remove_file(dir.join(opts.output_file));

    // NOTE: OK, this is a bit hacky, but it's a simple way to avoid running
    // the tests from tests/*.rs in parallel.
    //
    // I'm aware of `cargo test -- --test-threads=1` option, but I want to run
    // all tests with `cargo test` in parallel and limit the parallelism only
    // for tests that write to the file system, like the integration tests.
    let mut is_running = IS_RUNNING_MULTITHREAD.lock().unwrap();
    println!(
        "SINGLE THREAD: Is there another test running: {}",
        *is_running != 0
    );
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
        !std::path::Path::new(&dir.join(opts.output_file)).exists(),
       "the log file already exists, make sure to give an unique log file to avoid multiple writes to same file: {}",
       dir.join(opts.output_file).display()
    );

    let bin_path = dir.join("target/debug/fzz");
    let output_file = File::create(dir.join(opts.output_file)).expect("error log file");

    handler(
        Command::new(bin_path)
            .arg("-c")
            .arg(dir.join(opts.example_file))
            .stdout(Stdio::from(output_file)),
        File::open(dir.join(opts.output_file)).expect("failed to open file"),
    );

    std::fs::remove_file(dir.join(opts.output_file))
        .expect("failed to remove file after running test");
}

#[allow(dead_code)]
pub fn with_output<F>(output_file_path: &str, handler: F) -> ()
where
    F: FnOnce(&mut Command, File) -> (),
{
    let dir = env::current_dir().expect("error getting current directory");

    let _ = std::fs::remove_file(dir.join(output_file_path));

    // NOTE: OK, this is a bit hacky, but it's a simple way to avoid running
    // the tests from tests/*.rs in parallel.
    //
    // I'm aware of `cargo test -- --test-threads=1` option, but I want to run
    // all tests with `cargo test` in parallel and limit the parallelism only
    // for tests that write to the file system, like the integration tests.
    let mut is_running = IS_RUNNING_MULTITHREAD.lock().unwrap();
    println!(
        "SINGLE THREAD: Is there another test running: {}",
        *is_running != 0
    );
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
        !std::path::Path::new(&dir.join(output_file_path)).exists(),
       "the log file already exists, make sure to give an unique log file to avoid multiple writes to same file: {}",
       dir.join(output_file_path).display()
    );

    let bin_path = dir.join("target/debug/fzz");
    let output_file = File::create(dir.join(output_file_path)).expect("error log file");

    handler(
        Command::new(bin_path).stdout(Stdio::from(output_file)),
        File::open(dir.join(output_file_path)).expect("failed to open file"),
    );

    std::fs::remove_file(dir.join(output_file_path))
        .expect("failed to remove file after running test");
}

#[allow(dead_code)]
pub fn clean_output(output_file: &str) -> String {
    output_file
        .lines()
        .map(|line| {
            // This line prints the time so is not deterministic
            if line.starts_with("Funzzy: finished in") {
                return "Funzzy: finished in 0.0s";
            }

            line
        })
        .filter(|line| !line.contains("@@@@"))
        .collect::<Vec<&str>>()
        .join("\n")
        .to_string()
}
