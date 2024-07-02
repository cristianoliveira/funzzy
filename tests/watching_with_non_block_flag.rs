use std::io::prelude::*;

#[path = "./common/lib.rs"]
mod setup;

#[test]
fn test_it_cancel_current_running_task_when_something_change() {
    setup::with_example(
        setup::Options {
            output_file: "test_it_cancel_current_running_task_when_something_change.log",
            example_file: "examples/long-task.yaml",
        },
        |fzz_cmd, mut output_log| {
            let mut child = fzz_cmd
                .arg("--non-block")
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

                    output.contains("Running on init commands")
                        && output.contains("Started task long 2")
                },
                "No task in the example was configured with run_on_init {}",
                output
            );

            write_to_file!("examples/workdir/trigger-watcher.txt");
            // This should not trigger another restart
            write_to_file!("examples/workdir/another_ignored_file.foo");

            wait_until!(
                {
                    output_log
                        .read_to_string(&mut output)
                        .expect("failed to read from file");

                    output.contains("Task long 1 finished")
                },
                "Failed to find task 1: {}",
                output
            );

            write_to_file!("examples/workdir/trigger-watcher.txt");

            wait_until!(
                {
                    output_log
                        .read_to_string(&mut output)
                        .expect("failed to read from file");

                    // First on init
                    // Second when the file is changed
                    // Third when the file is changed again
                    output.match_indices("Started task long 2").count() == 3
                },
                "Failed find 3 instances of task list 2: {}",
                output
            );

            let clear_sign = "[H[J";
            assert_eq!(
                output.replace(clear_sign, ""),
                "Funzzy: Running on init commands.

Funzzy: bash examples/longtask.sh long 2 

Started task long 2
Long task running... 0

Funzzy: clear 

Long task running... 1
Task long 2 finished

Funzzy: bash examples/longtask.sh long 1 

Started task long 1
Long task running... 0
Task long 1 finished

Funzzy: bash examples/longtask.sh long 2 

Started task long 2
Long task running... 0

Funzzy: clear 

Long task running... 1
Task long 2 finished

Funzzy: bash examples/longtask.sh long 1 

Started task long 1
Long task running... 0
Task long 1 finished

Funzzy: bash examples/longtask.sh long 2 

Started task long 2
Long task running... 0
",
                "Output does not match expected: \n {}",
                output
            );
        },
    );
}
