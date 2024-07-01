use std::io::prelude::*; 

#[path = "./common/lib.rs"]
mod setup;

#[test]
fn test_it_gives_more_context_of_events_when_using_verbose() {
    setup::with_example(
        setup::Options {
            output_file: "test_it_gives_more_context_of_events_when_using_verbose.log",
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

                output.contains("Funzzy verbose: Events Ok")
                    && output.contains("examples/workdir/another_ignored_file.foo")
            }, "Failed to find Events Ok: {}", output);

            write_to_file!("examples/workdir/trigger-watcher.txt");

            wait_until!({
                output_log
                    .read_to_string(&mut output)
                    .expect("failed to read from file");

                output.contains("Funzzy verbose: Events Ok")
                    && output.contains("examples/workdir/trigger-watcher.txt")
            }, "Failed to find Events Ok: {}", output);

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
