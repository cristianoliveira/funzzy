use pretty_assertions::assert_eq;
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
            let mut child = fzz_cmd.arg("-V").spawn().expect("failed to spawn child");

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

            wait_until!(
                {
                    output_log
                        .read_to_string(&mut output)
                        .expect("failed to read from file");

                    output.contains("Funzzy verbose: Events Ok")
                        && output.contains("examples/workdir/ignored/modifyme.txt")
                },
                "Failed to find Events Ok: {}",
                output
            );

            write_to_file!("examples/workdir/another_ignored_file.foo");

            wait_until!(
                {
                    output_log
                        .read_to_string(&mut output)
                        .expect("failed to read from file");

                    output.contains("something changed in workdir!")
                },
                "Failed to find 'something changed in workdir!': {}",
                output
            );

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

            wait_until!(
                {
                    output_log
                        .read_to_string(&mut output)
                        .expect("failed to read from file");

                    output.contains("Funzzy: Watching...")
                },
                "OUTPUT: {}",
                output
            );

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

            assert_eq!(
                setup::clean_output(&output),
                "
[2J
Funzzy: echo first 

first

Funzzy: echo second 

second

Funzzy: echo complex | sed s/complex/third/g 

third

Funzzy: echo 'something changed in workdir!' 

something changed in workdir!
Funzzy results ----------------------------
Success; Completed: 4; Failed: 0; Duration: 0.0000s"
            );
        },
    );
}

#[test]
#[cfg(feature = "test-integration-file-system")]
fn accepts_full_or_relativepaths() {
    setup::with_example(
        setup::Options {
            output_file: "accepts_full_or_relativepaths.log",
            example_file: "examples/tasks-with-absolute-paths.yml",
        },
        |fzz_cmd, mut output_log| {
            // Initialize the files
            // NOTE: To debug if the setup is correct
            // shell!("ls la /tmp/");

            write_to_file!("/tmp/fzz/accepts_full_or_relativepaths.txt");
            write_to_file!("/tmp/fzz/accepts_full_or_relativepaths2.txt");
            write_to_file!("/tmp/fzz/accepts_full_or_relativepaths3.txt");

            let mut child = fzz_cmd
                .arg("-t")
                .arg("@valid")
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

                    output.contains("Funzzy verbose: Verbose mode enabled.")
                        && output.contains("Watching...")
                },
                "fzz not started in verbose {}",
                output
            );

            write_to_file!("/tmp/fzz/accepts_full_or_relativepaths.txt");
            write_to_file!("/tmp/fzz/accepts_full_or_relativepaths2.txt");
            write_to_file!("/tmp/fzz/accepts_full_or_relativepaths3.txt");
            write_to_file!("examples/workdir/trigger-watcher.txt");
            write_to_file!("examples/workdir/ignored/modify.txt");

            wait_until!(
                {
                    output_log
                        .read_to_string(&mut output)
                        .expect("failed to read from file");

                    output
                        .split("\n")
                        .filter(|line| {
                            line.starts_with("Funzzy verbose: Triggered")
                                && (line.contains("/tmp/fzz/accepts_full_or_relativepaths.txt")
                                    || line.contains("/tmp/fzz/accepts_full_or_relativepaths2.txt")
                                    || line.contains("examples/workdir/trigger-watcher.txt"))
                        })
                        .count()
                        == 3
                        && output
                            .split("\n")
                            .find(|line| {
                                line.starts_with("Funzzy verbose: Triggered")
                                    && (line
                                        .contains("/tmp/fzz/accepts_full_or_relativepaths3.txt")
                                        || line.contains("examples/workdir/ignored/modify.txt"))
                            })
                            .is_none()
                },
                "triggered task that was not in watch list {}",
                output
            );
        },
    );
}

#[test]
fn fails_with_unkown_paths() {
    setup::with_example(
        setup::Options {
            output_file: "fails_with_unkown_paths.log",
            example_file: "examples/tasks-with-absolute-paths.yml",
        },
        |fzz_cmd, mut output_log| {
            let mut child = fzz_cmd
                .arg("-t")
                .arg("@invalid")
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

                    output.contains("Funzzy verbose: Verbose mode enabled.")
                },
                "fzz not started in verbose {}",
                output
            );

            wait_until!(
                {
                    output_log
                        .read_to_string(&mut output)
                        .expect("failed to read from file");

                    output
                        .contains("Funzzy warning: unknown file/directory: '/tmp/fzz/unknown.txt'")
                },
                "expected output contain error explanation but got {}",
                output
            );

            write_to_file!("examples/workdir/trigger-watcher.txt");
            wait_until!(
                {
                    output_log
                        .read_to_string(&mut output)
                        .expect("failed to read from file");

                    output.contains("examples/workdir/trigger-watcher.txt")
                        && output.contains("Funzzy results")
                },
                "expected output contain error explanation but got {}",
                output
            );
        },
    );
}
