use std::fs::File;
use std::io::prelude::*;
use std::process::{Command, Stdio};

#[path = "./common/lib.rs"]
mod setup;

#[test]
fn test_it_allows_run_arbitrary_commans_with_by_piping_files() {
    let test_log_file = "test_it_allows_run_arbitrary_commans_with_by_piping_files.log";
    setup::with_output(test_log_file, |fzz_cmd, mut output_log| {
        let files = Command::new("find")
            .arg(".")
            .arg("-name")
            .arg("*.txt")
            .stdout(Stdio::piped())
            .spawn()
            .expect("failed to run find");

        let mut child = fzz_cmd
            .arg("echo 'running arbitrary command'")
            .stdin(files.stdout.expect("failed to open stdin"))
            .spawn()
            .expect("Failed to spawn grep command");

        defer!({
            child.kill().expect("failed to kill child");
        });

        let mut output = String::new();
        wait_until!(
            {
                output_log
                    .read_to_string(&mut output)
                    .expect("failed to read from file");

                output.contains("Funzzy: Watching...")
            },
            "Failed to find Funzzy results"
        );

        write_to_file!("examples/workdir/another_ignored_file.foo");
        write_to_file!("examples/workdir/trigger-watcher.txt");

        wait_until!(
            {
                output_log
                    .read_to_string(&mut output)
                    .expect("failed to read from file");

                output.contains("running arbitrary command")
            },
            "Failed to find Funzzy results"
        );

        write_to_file!("examples/workdir/another_ignored_file.foo");
        write_to_file!("examples/workdir/trigger-watcher.txt");

        wait_until!(
            {
                output_log
                    .read_to_string(&mut output)
                    .expect("failed to read from file");

                output.split("running arbitrary command").count() > 1
                    && output.split("Funzzy results").count() == 2
            },
            "Failed to find Funzzy results"
        );
    });
}

#[test]
fn test_it_allows_templates_in_arbitrary_commands() {
    setup::with_output(
        "test_it_allows_templates_in_arbitrary_commands.log",
        |fzz_cmd, mut output_log| {
            let files = Command::new("find")
                .arg(".")
                .arg("-name")
                .arg("*.txt")
                .stdout(Stdio::piped())
                .spawn()
                .expect("failed to run find");

            let mut child = fzz_cmd
                .arg("echo 'this file changed: {{filepath}}'")
                .stdin(files.stdout.expect("failed to open stdin"))
                .spawn()
                .expect("Failed to spawn grep command");

            defer!({
                child.kill().expect("failed to kill child");
            });

            let mut output = String::new();
            wait_until!(
                {
                    output_log
                        .read_to_string(&mut output)
                        .expect("failed to read from file");

                    output.contains("Funzzy: Watching...")
                },
                "Failed to find Funzzy results"
            );

            write_to_file!("examples/workdir/another_ignored_file.foo");
            write_to_file!("examples/workdir/trigger-watcher.txt");

            wait_until!(
                {
                    output_log
                        .read_to_string(&mut output)
                        .expect("failed to read from file");

                    let dir = std::env::current_dir().expect("failed to get current dir");
                    output.contains(&format!(
                        "this file changed: {}",
                        dir.join("examples/workdir/trigger-watcher.txt")
                            .to_str()
                            .expect("failed to convert path to string")
                    )) && output.contains("All tasks finished successfully.")
                },
                "Could not find the file path in the output: {}",
                output
            );
        },
    );
}
