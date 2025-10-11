use assert_cmd::Command;
use predicates::prelude::predicate;
use pretty_assertions::assert_eq;
use std::io::prelude::*;

#[path = "./common/lib.rs"]
mod setup;

#[test]
fn test_it_filter_tasks_with_target_flag() {
    setup::with_example(
        setup::Options {
            output_file: "test_it_filter_tasks_with_target_flag.log",
            example_file: "examples/tasks-with-tags-to-filter.yml",
        },
        |fzz_cmd, mut output_log| {
            let mut child = fzz_cmd
                .arg("--target")
                .arg("@quick")
                .spawn()
                .expect("failed to spawn child");

            defer!({
                child.kill().expect("failed to kill child");
            });

            let mut output = String::new();

            wait_until!(
                {
                    output_log
                        .read_to_string(&mut output)
                        .expect("failed to read from file");

                    output.contains("Watching...")
                },
                "Funzzy failed to watch {}",
                output
            );

            write_to_file!("examples/workdir/trigger-watcher.txt");

            wait_until!(
                {
                    output_log
                        .read_to_string(&mut output)
                        .expect("failed to read from file");

                    output.contains("Funzzy results")
                },
                "Failed to find Funzzy results: {}",
                output
            );

            assert_eq!(
                setup::clean_output(&output),
                "Funzzy: Watching...

[2J
Funzzy: echo 'quick tests' 

quick tests

Funzzy: echo 'another quick task' 

another quick task

Funzzy: echo 'quick lint' 

quick lint
Funzzy results ----------------------------
Success; Completed: 3; Failed: 0; Duration: 0.0000s"
            );
        },
    );
}

#[test]
fn test_it_list_the_available_tasks_when_nothing_matches() {
    let mut cmd = Command::cargo_bin("fzz").expect("failed to get cargo bin");
    cmd.env("FUNZZY_COLORED", "false")
        .arg("-t")
        .arg("unknown_task_name")
        .arg("-c")
        .arg("examples/tasks-with-tags-to-filter.yml")
        .assert()
        .failure()
        .stdout(predicate::str::contains(
            "Error: No target found for \'unknown_task_name\'
Available tasks
  - run my test @quick
  - run my build
  - run my lint @quick",
        ));
}

#[test]
fn test_it_list_the_available_tasks_flag_is_empty() {
    let mut cmd = Command::cargo_bin("fzz").expect("failed to get cargo bin");
    cmd.env("FUNZZY_COLORED", "false")
        .arg("-c")
        .arg("examples/tasks-with-tags-to-filter.yml")
        .arg("-t")
        .assert()
        .success()
        .stdout(predicate::str::contains(
            "Funzzy: `--target` help
Available tasks
  - run my test @quick
  - run my build
  - run my lint @quick

Usage `fzz -t <text_contain_in_task>`",
        ));
}
