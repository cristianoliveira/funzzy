use std::io::prelude::*;

#[path = "./common/lib.rs"]
mod setup;

#[test]
fn test_it_writes_output_to_log_file() {
    setup::with_example(
        setup::Options {
            output_file: "test_it_writes_output_to_log_file.console.log",
            example_file: "examples/simple-case.yml",
        },
        |fzz_cmd, mut output_log| {
            let log_path = std::env::temp_dir().join("funzzy_integration_output.log");
            let _ = std::fs::remove_file(&log_path);

            let mut child = fzz_cmd
                .arg("--log-file")
                .arg(&log_path)
                .spawn()
                .expect("failed to spawn child");

            defer!({
                let _ = child.kill();
                let _ = std::fs::remove_file(&log_path);
            });

            let mut console_output = String::new();

            wait_until!(
                {
                    output_log
                        .read_to_string(&mut console_output)
                        .expect("failed to read console output");

                    console_output.contains("Watching...")
                },
                "Funzzy has not started watching. Output: {}",
                console_output
            );

            console_output.truncate(0);

            write_to_file!("examples/workdir/trigger-watcher.txt");

            wait_until!(
                {
                    output_log
                        .read_to_string(&mut console_output)
                        .expect("failed to read console output");

                    console_output.contains("something changed in workdir!")
                },
                "Failed to observe command output. Console: {}",
                console_output
            );

            wait_until!(
                {
                    match std::fs::read_to_string(&log_path) {
                        Ok(contents) => {
                            contents.contains("Funzzy: echo 'something changed in workdir!'")
                                && contents.contains("something changed in workdir!")
                        }
                        Err(_) => false,
                    }
                },
                "Log file missing expected output"
            );
        },
    );
}
