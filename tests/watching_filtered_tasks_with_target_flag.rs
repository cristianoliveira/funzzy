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
All tasks finished successfully. Finished in 0.0s"
            );
        },
    );
}

#[test]
fn test_it_list_the_available_tasks_when_nothing_matches() {
    setup::with_example(
        setup::Options {
            output_file: "test_it_list_the_available_tasks_when_nothing_matches.log",
            example_file: "examples/tasks-with-tags-to-filter.yml",
        },
        |fzz_cmd, mut output_log| {
            let mut child = fzz_cmd
                .arg("--target")
                .arg("@something_not_in_the_list")
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

                    output.contains("Finished there is no task to run")
                },
                "Funzzy failed to watch {}",
                output
            );

            assert_eq!(
                output,
                "Funzzy: No target found for '@something_not_in_the_list'

Available targets:
  run my test @quick
  run my build
  run my lint @quick

Finished there is no task to run.
",
                "failed to find the expected output: {}",
                output
            );
        },
    );
}
