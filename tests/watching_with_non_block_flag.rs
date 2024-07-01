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
                },
                "Funzzy failed to watch {}",
                output
            );

            wait_until!(
                {
                    output_log
                        .read_to_string(&mut output)
                        .expect("failed to read from file");

                    // NOTE: task list 4 runs on init
                    output.contains("Started task list 4")
                        && output.contains("Long task running... 0")
                        && output.contains("Long task running... 1")
                        && output.match_indices("Started task list 4").count() == 1
                },
                "No task in the example was configured with run_on_init {}",
                output
            );

            write_to_file!("examples/workdir/trigger-watcher.txt");

            wait_until!(
                {
                    output_log
                        .read_to_string(&mut output)
                        .expect("failed to read from file");

                    output.contains("Long task running... 1")
                        && output.contains("Long task running... 2")
                        && output.match_indices("Started task list 4").count() == 2
                        && output.match_indices("Started task list 3").count() == 1
                },
                "Failed to find Funzzy results: {}",
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

                    output.contains("Long task running... 1")
                        && output.contains("Long task running... 2")
                        && output.match_indices("Started task list 4").count() == 3
                },
                "Failed to find Funzzy results: {}",
                output
            );

            let clear_sign = "\u{1b}[H\u{1b}[J";
            assert_eq!(
                output.replace(clear_sign, ""),
                "Funzzy: Running on init commands.

Funzzy: task bash examples/longtask.sh list 4 

Funzzy: Watching...
Started task list 4
Long task running... 0
Long task running... 1

Funzzy: clear 

Long task running... 2
Long task running... 3
Task list 4 finished

Funzzy: task bash examples/longtask.sh list 4 

Started task list 4
Long task running... 0
Long task running... 1
Long task running... 2
Long task running... 3
Task list 4 finished

Funzzy: task bash examples/longtask.sh list 3 

Started task list 3
Long task running... 0

Funzzy: clear 

Long task running... 1
Long task running... 2
Task list 3 finished

Funzzy: task bash examples/longtask.sh list 4 

Started task list 4
Long task running... 0
",
                "Output does not match expected: \n {}",
                output
            );
        },
    );
}
