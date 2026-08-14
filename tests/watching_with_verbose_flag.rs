use std::io::prelude::*;

#[path = "./common/lib.rs"]
mod setup;

/// Asserts the new typed lifecycle diagnostics (TASK-0023): one `Funzzy
/// debug:` record per event batch path, then the matched/ignored decisions
/// with stable vocabulary — never raw `Debug` dumps.
#[test]
fn test_it_gives_more_context_of_events_when_using_verbose() {
    setup::with_example(
        setup::Options {
            output_file: "test_it_gives_more_context_of_events_when_using_verbose.log",
            example_file: "examples/simple-case.yml",
        },
        |fzz_cmd, mut output_log, fixture| {
            let mut child = fzz_cmd.arg("-v").spawn().expect("failed to spawn child");

            defer!({
                child.kill().expect("failed to kill child");
            });

            let mut output = String::new();

            wait_until!(
                {
                    output_log
                        .read_to_string(&mut output)
                        .expect("failed to read from file");

                    output.contains("Funzzy debug:") && output.contains("Watching...")
                },
                "Funzzy has not been started with verbose mode {}",
                output
            );

            output.truncate(0);

            write_to_file!(fixture.join("examples/workdir/ignored/modifyme.txt"));

            wait_until!(
                {
                    output_log
                        .read_to_string(&mut output)
                        .expect("failed to read from file");

                    output.contains("source=filesystem")
                        && output.contains("examples/workdir/ignored/modifyme.txt")
                },
                "Failed to find the event record: {}",
                output
            );

            // The event record carries the observed path. In this config the
            // path also matches the workdir rule (no ignore), so the batch
            // triggers it; the ignoring task itself must never run (asserted
            // below).
            output.truncate(0);

            write_to_file!(fixture.join("examples/workdir/another_ignored_file.foo"));

            wait_until!(
                {
                    output_log
                        .read_to_string(&mut output)
                        .expect("failed to read from file");

                    output.contains("source=filesystem")
                        && output.contains("examples/workdir/another_ignored_file.foo")
                },
                "Failed to find the event record: {}",
                output
            );

            output.truncate(0);

            write_to_file!(fixture.join("examples/workdir/trigger-watcher.txt"));

            wait_until!(
                {
                    output_log
                        .read_to_string(&mut output)
                        .expect("failed to read from file");

                    output.contains("source=filesystem")
                        && output.contains("examples/workdir/trigger-watcher.txt")
                },
                "Failed to find the event record: {}",
                output
            );

            // The matched decision names the task and effective rule.
            assert!(
                output.contains("decision=matched"),
                "matched decision must be recorded: {}",
                output
            );

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
            assert!(
                !output.contains("Events Ok"),
                "raw Debug event dumps must be replaced by typed records. \n Output: {}",
                output
            );
        },
    );
}
