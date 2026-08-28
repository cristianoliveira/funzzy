use assert_cmd::cargo;
use predicates::prelude::*;
use pretty_assertions::assert_eq;
use std::io::prelude::*;

#[path = "./common/lib.rs"]
mod setup;

#[test]
fn test_it_filter_tasks_with_watch_target() {
    setup::with_example(
        setup::Options {
            output_file: "test_it_filter_tasks_with_watch_target.log",
            example_file: "examples/jobs-with-tags-to-filter.yml",
        },
        |fzz_cmd, mut output_log, fixture| {
            let mut child = fzz_cmd
                .args(["watch", "@quick"])
                .spawn()
                .expect("failed to spawn child");

            defer!({
                child.kill().expect("failed to kill child");
                let _ = child.wait();
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

            write_to_file!(fixture.join("examples/workdir/trigger-watcher.txt"));

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
                setup::strip_ansi_codes(&setup::clean_output(&output)),
                "Funzzy: Watching...

[2J
Funzzy: echo 'quick tests' 

quick tests

Funzzy: echo 'another quick task' 

another quick task

Funzzy: echo 'quick lint' 

quick lint
Funzzy results ----------------------------
JOB                 RESULT  DURATION
run my test @quick  passed  0ms
run my lint @quick  passed  0ms
Success; Completed: 3; Failed: 0; Duration: 0.0000s"
            );
        },
    );
}

#[test]
fn test_it_list_the_available_tasks_when_nothing_matches() {
    let mut cmd = cargo::cargo_bin_cmd!("fzz");
    cmd.env("FUNZZY_COLORED", "false")
        .arg("-c")
        .arg("examples/jobs-with-tags-to-filter.yml")
        .args(["watch", "unknown_task_name"])
        .assert()
        .failure()
        .stdout(predicate::str::contains(
            "Error: No target found for 'unknown_task_name'
Available jobs
  - run my test @quick
    change: examples/workdir/*.txt
  - run my build
    change: examples/workdir/*.txt
    run_on_init: true
  - run my lint @quick
    change: examples/workdir/*.txt",
        ));
}

#[test]
fn test_list_subcommand_lists_available_tasks() {
    let mut cmd = cargo::cargo_bin_cmd!("fzz");
    cmd.env("FUNZZY_COLORED", "false")
        .arg("-c")
        .arg("examples/jobs-with-tags-to-filter.yml")
        .arg("list")
        .assert()
        .success()
        .stdout(predicate::str::contains(
            "Available jobs
  - run my test @quick
    change: examples/workdir/*.txt
  - run my build
    change: examples/workdir/*.txt
    run_on_init: true
  - run my lint @quick
    change: examples/workdir/*.txt",
        ))
        .stdout(predicate::str::contains("Usage").not());
}
