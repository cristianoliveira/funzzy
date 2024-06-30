use std::io::prelude::*;
use std::{
    fs::File,
    thread::sleep,
    time::Duration,
};

#[path = "./common/lib.rs"]
mod setup;

#[test]
fn test_it_is_not_triggered_by_ignored_files() {
    setup::with_example(
        setup::Options {
            log_file: "test_it_is_not_triggered_by_ignored_files.log",
            example_file: "examples/simple-case.yml",
        },
        |fzz_cmd, mut log| {
            let mut child = fzz_cmd
                .arg("-V")
                .arg("-t")
                .arg("ignoring rules")
                .spawn()
                .expect("failed to spawn child");

            defer!({
                child.kill().expect("failed to kill child");
            });

            let mut output = String::new();

            wait_until!(
                {
                    log.read_to_string(&mut output)
                        .expect("failed to read from file");

                    output.contains("Funzzy verbose") && output.contains("Watching...")
                },
                "Funzzy has not been started with verbose mode {}",
                output
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
        },
    );
}

#[test]
fn test_it_watch_files_and_execute_configured_commands() {
    setup::with_example(
        setup::Options {
            example_file: "examples/simple-case.yml",
            log_file: "test_it_watch_files_and_execute_configured_commands.log",
        },
        |fzz_cmd, mut logger| {
            let mut child = fzz_cmd.spawn().expect("failed to spawn process");
            let mut output = String::new();
            defer!({
                child.kill().expect("failed to close process");
            });

            wait_until!({
                logger
                    .read_to_string(&mut output)
                    .expect("failed to read from file");

                output.contains("Watching ...")
            });

            output.truncate(0);

            write_to_file!("examples/workdir/trigger-watcher.txt");

            wait_until!(
                {
                    logger
                        .read_to_string(&mut output)
                        .expect("failed to read from file");

                    output.contains("Funzzy results")
                },
                "OUTPUT: {}",
                output
            );

            let clear_noise = "[H[J";
            assert_eq!(
                output.replace(clear_noise, ""),
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
        },
    );
}
