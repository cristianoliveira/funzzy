use std::io::prelude::*;
use std::{
    env,
    fs::File,
    process::{Command, Stdio},
    thread::sleep,
    time::Duration,
};

#[path = "./common/macros.rs"]
mod common_macros;

#[test]
fn test_it_is_not_triggered_by_ignored_files() {
    let test_log = "test_it_is_not_triggered_by_ignored_files.log";

    let dir = env::current_dir().expect("failed to get current directory");
    let bin_path = dir.join("target/debug/fzz");

    let output_log = File::create(dir.join(test_log)).expect("error log file");
    output_log.set_len(0).expect("failed to truncate file");
    let stdio = Stdio::from(output_log);

    let mut child = Command::new(bin_path)
        .arg("-V")
        .arg("-c")
        .arg(dir.join("examples/simple-case.yml"))
        .arg("-t")
        .arg("ignoring rules")
        .stdout(stdio)
        .spawn()
        .expect("failed to spawn child");

    defer!({
        child.kill().expect("failed to kill child");
        if let Err(e) = std::fs::remove_file(dir.join(test_log)) {
            assert!(false, "Failed while removing log file: {:?}", e);
        }
    });

    let mut output = String::new();
    let mut log = File::open(dir.join(test_log)).expect("failed to open file");

    wait_until!(
        {
            log.read_to_string(&mut output)
                .expect("failed to read from file");

            output.contains("Funzzy verbose") && output.contains("Watching...")
        },
        "Funzzy has not been started with verbose mode"
    );

    output.truncate(0);

    write_to_file!("examples/workdir/ignored/modifyme.txt");

    sleep(Duration::from_secs(2));

    wait_until!({
        sleep(Duration::from_millis(100));
        log.read_to_string(&mut output)
            .expect("failed to read from file");

        output.contains("Funzzy verbose: Events Ok")
            && output.contains("examples/workdir/ignored/modifyme.txt")
    });

    assert!(
        !output.contains("Triggered by"),
        "triggered an ignored rule. \n Output: {}",
        output
    );

    write_to_file!("examples/workdir/another_ignored_file.foo");

    assert!(
        !output.contains("Triggered by"),
        "triggered an ignored rule. \n Output: {}",
        output
    );
}

#[test]
fn test_it_watch_a_simple_case() {
    let test_log = "test_it_watch_a_simple_case.log";
    let clear_char = "[H[J";

    let dir = env::current_dir().expect("failed to get current directory");
    let bin_path = dir.join("target/debug/fzz");

    let _ = std::fs::remove_file(dir.join(test_log));
    let output_log = File::create(dir.join(test_log)).expect("error log file");
    output_log.set_len(0).expect("failed to truncate file");
    let stdio = Stdio::from(output_log);

    let mut child = Command::new(bin_path)
        .arg("-c")
        .arg(dir.join("examples/simple-case.yml"))
        .arg("-t")
        .arg("list of commands")
        .stdout(stdio)
        .spawn()
        .expect("failed to spawn child");

    defer!({
        child.kill().expect("failed to kill child");
        if let Err(e) = std::fs::remove_file(dir.join(test_log)) {
            assert!(false, "Failed while removing log file: {:?}", e);
        }
    });

    let mut output = String::new();
    let mut log = File::open(dir.join(test_log)).expect("failed to open file");

    // wait for the watcher to start
    wait_until!({
        sleep(Duration::from_millis(100));
        log.read_to_string(&mut output)
            .expect("failed to read from file");

        output.contains("Watching...")
    });

    output.truncate(0);

    write_to_file!("examples/workdir/trigger-watcher.txt");

    wait_until!(
        {
            log.read_to_string(&mut output)
                .expect("failed to read from file");

            output.contains("Funzzy results")
        },
        "Output does not contain 'Funzzy results'"
    );

    assert_eq!(
        output.replace(clear_char, ""),
        "
Funzzy: clear 


Funzzy: echo first 

first

Funzzy: echo second 

second

Funzzy: echo complex | sed s/complex/third/g 

third
Funzzy results ----------------------------
All tasks finished successfully.
"
    );
}
