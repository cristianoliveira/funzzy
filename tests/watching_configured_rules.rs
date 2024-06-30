use std::io::prelude::*; 

#[path = "./common/lib.rs"]
mod setup;

#[test]
fn test_it_is_not_triggered_by_ignored_files() {
    setup::with_example(
        setup::Options {
            output_file: "test_it_is_not_triggered_by_ignored_files.log",
            example_file: "examples/simple-case.yml",
        },
        |fzz_cmd, mut output_log| {
            let mut child = fzz_cmd
                .arg("-V")
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

                    output.contains("Funzzy verbose") && output.contains("Watching...")
                },
                "Funzzy has not been started with verbose mode {}",
                output
            );

            output.truncate(0);

            write_to_file!("examples/workdir/ignored/modifyme.txt");

            wait_until!({
                output_log
                    .read_to_string(&mut output)
                    .expect("failed to read from file");

                output.contains("Funzzy verbose: Events Ok")
                    && output.contains("examples/workdir/ignored/modifyme.txt")
            }, "Failed to find Events Ok: {}", output);

            write_to_file!("examples/workdir/another_ignored_file.foo");

            wait_until!({
                output_log
                    .read_to_string(&mut output)
                    .expect("failed to read from file");

                output.contains("something changed in workdir!")
            }, "Failed to find 'something changed in workdir!': {}", output);

            assert!(
                !output.contains("should not trigger when modifying files in ignored files"),
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
            output_file: "test_it_watch_files_and_execute_configured_commands.log",
        },
        |fzz_cmd, mut output_log| {
            let mut child = fzz_cmd.spawn().expect("failed to spawn process");
            let mut output = String::new();
            defer!({
                child.kill().expect("failed to close process");
            });

            wait_until!({
                output_log
                    .read_to_string(&mut output)
                    .expect("failed to read from file");

                output.contains("Funzzy: Watching...")
            }, "OUTPUT: {}", output);

            output.truncate(0);

            write_to_file!("examples/workdir/trigger-watcher.txt");

            wait_until!(
                {
                    output_log
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

Funzzy: echo 'something changed in workdir!' 

something changed in workdir!
Funzzy results ----------------------------
All tasks finished successfully.

Funzzy: clear 


Funzzy: echo 'something changed in workdir!' 

something changed in workdir!
Funzzy results ----------------------------
All tasks finished successfully.
"
            );
        },
    );
}
