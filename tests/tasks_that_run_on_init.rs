use std::collections::HashMap;
use std::io::prelude::*;

// use pretty_assertions::assert_eq;

#[path = "./common/lib.rs"]
mod setup;

#[test]
fn test_it_executes_tasks_on_init_when_configured() {
    setup::with_example(
        setup::Options {
            output_file: "test_it_executes_tasks_on_init_when_configured.log",
            example_file: "examples/list-of-tasks-run-on-init.yml",
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

                    output.contains("Funzzy results")
                },
                "No task in the example was configured with run_on_init {}",
                output
            );

            assert_eq!(
                setup::clean_output(&output),
                "Funzzy: Running on init commands.

Funzzy: echo 'running on init first' 

running on init first

Funzzy: echo \"run on init sencod\" 

run on init sencod

Funzzy: echo \"only run on init\" 

only run on init
Funzzy results ----------------------------
Success; Completed: 3; Failed: 0; Durantion: 0.0000s"
            );

            // FIXME: this should not be needed sleep 5s
            std::thread::sleep(std::time::Duration::from_secs(5));
            write_to_file!("examples/workdir/trigger-watcher.txt");

            wait_until!(
                {
                    output_log
                        .read_to_string(&mut output)
                        .expect("failed to read from file");

                    output.contains("should not run on init but on change")
                },
                "OUTPUT: {}",
                output
            );
        },
    )
}

#[test]
fn test_it_does_not_executes_tasks_on_init_when_no_run_on_init_flag() {
    setup::with_example(
        setup::Options {
            output_file: "test_it_does_not_executes_tasks_on_init_when_no_run_on_init_flag.log",
            example_file: "examples/list-of-tasks-run-on-init.yml",
        },
        |fzz_cmd, mut output_log| {
            let mut child = fzz_cmd
                .arg("--no-run-on-init")
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

                    !output.contains("Running on init commands") && output.contains("Watching...")
                },
                "No task in the example was configured with run_on_init {}",
                output
            );

            // FIXME: this should not be needed sleep 5s
            std::thread::sleep(std::time::Duration::from_secs(5));
            write_to_file!("examples/workdir/trigger-watcher.txt");

            wait_until!(
                {
                    output_log
                        .read_to_string(&mut output)
                        .expect("failed to read from file");

                    output.contains("should not run on init but on change")
                },
                "OUTPUT: {}",
                output
            );

            assert_eq!(
                setup::clean_output(&output),
                "Funzzy: Watching...

[2J
Funzzy: echo 'running on init first' 

running on init first

Funzzy: echo \"should not run on init but on change\" 

should not run on init but on change

Funzzy: echo \"run on init sencod\" 

run on init sencod
Funzzy results ----------------------------
Success; Completed: 3; Failed: 0; Durantion: 0.0000s"
            );
        },
    )
}
