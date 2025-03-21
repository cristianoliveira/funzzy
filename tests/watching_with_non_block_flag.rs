use std::collections::HashMap;
use std::io::prelude::*;

use crate::setup::CLEAR_SCREEN;

#[path = "./common/lib.rs"]
mod setup;

#[test]
fn test_it_cancel_current_running_task_when_something_change() {
    setup::with_example(
        setup::Options {
            output_file: "test_it_cancel_current_running_task_when_something_change.log",
            example_file: "examples/tasks-with-long-running-commands.yaml",
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

                    // See it in `examples/longtask.sh`
                    // and also in `src/stdout.rs`
                    output.match_indices(CLEAR_SCREEN).count() == 1
                },
                "Failed to find a clear screen sign: {}",
                output
            );

            write_to_file!("examples/workdir/trigger-watcher.txt");

            wait_until!(
                {
                    output_log
                        .read_to_string(&mut output)
                        .expect("failed to read from file");

                    // See it in `examples/longtask.sh`
                    // and also in `src/stdout.rs`
                    output.match_indices(CLEAR_SCREEN).count() == 2
                },
                "Failed to find 2 clear screen signs: {}",
                output
            );

            let expected = "Funzzy: Running on init commands.

Funzzy: bash examples/longtask.sh long 2 

Started task long 2
Long task running... 0

[2J
Funzzy: bash examples/longtask.sh long 1 

Started task long 1
Long task running... 0

[2J
Funzzy: bash examples/longtask.sh long 1 

Started task long 1
Long task running... 0
";

            assert_eq!(
                output, expected,
                "Output:\n{} ------ \n\nExpected:\n{}",
                output, expected,
            );
        },
    );
}

#[test]
fn test_it_cancel_current_running_task_when_something_change_with_env() {
    setup::with_env(
        HashMap::from([("FUNZZY_NON_BLOCK".to_string(), "true".to_string())]),
        || {
            setup::with_example(
                setup::Options {
                    output_file:
                        "test_it_cancel_current_running_task_when_something_change_with_env.log",
                    example_file: "examples/tasks-with-long-running-commands.yaml",
                },
                |fzz_cmd, mut output_log| {
                    let mut child = fzz_cmd.spawn().expect("failed to spawn child");

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

                            // See it in `examples/longtask.sh`
                            // and also in `src/stdout.rs`
                            output.match_indices(CLEAR_SCREEN).count() == 1
                        },
                        "Failed to find a clear screen sign: {}",
                        output
                    );

                    write_to_file!("examples/workdir/trigger-watcher.txt");

                    wait_until!(
                        {
                            output_log
                                .read_to_string(&mut output)
                                .expect("failed to read from file");

                            // See it in `examples/longtask.sh`
                            // and also in `src/stdout.rs`
                            output.match_indices(CLEAR_SCREEN).count() == 2
                        },
                        "Failed to find 2 clear screen signs: {}",
                        output
                    );

                    let expected = "Funzzy: Running on init commands.

Funzzy: bash examples/longtask.sh long 2 

Started task long 2
Long task running... 0

[2J
Funzzy: bash examples/longtask.sh long 1 

Started task long 1
Long task running... 0

[2J
Funzzy: bash examples/longtask.sh long 1 

Started task long 1
Long task running... 0
";

                    assert_eq!(
                        output, expected,
                        "Output:\n{} ------ \n\nExpected:\n{}",
                        output, expected,
                    );
                },
            );
            Ok(())
        },
    )
    .expect("failed to set env");
}
