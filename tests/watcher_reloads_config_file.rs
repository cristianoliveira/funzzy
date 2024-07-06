use std::io::prelude::*;

#[path = "./common/lib.rs"]
mod setup;

#[test]
fn it_terminates_the_current_running_watcher_when_config_changes() {
    setup::with_example(
        setup::Options {
            output_file: "it_terminates_the_current_running_watcher_when_config_changes.log",
            example_file: "examples/reload-config-example.yml",
        },
        |fzz_cmd, mut output_log| {
            let mut child = fzz_cmd
                .arg("--non-block")
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

                    output.contains("The config file has changed")
                        && output.contains("Terminating watcher...")
                },
                "No task in the example was configured with run_on_init {}",
                output
            );
        },
    );
}
