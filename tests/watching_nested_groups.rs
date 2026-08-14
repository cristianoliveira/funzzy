use pretty_assertions::assert_eq;
use std::io::prelude::*;

#[path = "./common/lib.rs"]
mod setup;

/// Drains the shared watcher log until no new output appears for `quiet_ms`.
/// `write_to_file!` emits create + write events that the debouncer may split
/// into two batches under load, running the task twice; the duplicate echo
/// must land in the phase that triggered it, not leak into the next phase's
/// assertion window after `output.truncate(0)`.
fn drain_until_quiet(output_log: &mut std::fs::File, output: &mut String, quiet_ms: u64) {
    loop {
        let before = output.len();
        output_log
            .read_to_string(output)
            .expect("failed to read from file");
        if output.len() == before {
            std::thread::sleep(std::time::Duration::from_millis(quiet_ms));
            let before_idle = output.len();
            output_log
                .read_to_string(output)
                .expect("failed to read from file");
            if output.len() == before_idle {
                break;
            }
        }
    }
}

#[test]
#[cfg(feature = "test-integration")]
fn test_nested_groups_watch_different_patterns() {
    setup::with_example(
        setup::Options {
            output_file: "test_nested_groups_watch_different_patterns.log",
            example_file: "examples/test-nested-groups.yml",
        },
        |fzz_cmd, mut output_log, fixture| {
            // Create directories for test
            std::fs::create_dir_all(fixture.join("examples/workdir/frontend")).ok();
            std::fs::create_dir_all(fixture.join("examples/workdir/backend")).ok();
            std::fs::create_dir_all(fixture.join("examples/workdir/regular")).ok();

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
                "Funzzy did not start. OUTPUT: {}",
                output
            );

            // Test 1: Trigger frontend group
            write_to_file!(fixture.join("examples/workdir/frontend/test.js"));

            wait_until!(
                {
                    output_log
                        .read_to_string(&mut output)
                        .expect("failed to read from file");

                    output.contains("frontend task executed")
                },
                "Frontend task did not execute. OUTPUT: {}",
                output
            );

            assert!(
                output.contains("frontend task executed"),
                "Frontend task should execute"
            );
            assert!(
                !output.contains("backend task executed"),
                "Backend task should not execute. OUTPUT: {}",
                output
            );
            assert!(
                !output.contains("regular task executed"),
                "Regular task should not execute. OUTPUT: {}",
                output
            );

            // Drain any duplicate echo from the debounce split before the
            // next phase starts, so phase windows stay isolated. The quiet
            // window must exceed the watcher debounce timeout (1s) so a
            // debounce-split duplicate run cannot leak into the next phase.
            drain_until_quiet(&mut output_log, &mut output, 1_500);
            output.truncate(0);

            // Test 2: Trigger backend group
            write_to_file!(fixture.join("examples/workdir/backend/test.rs"));

            wait_until!(
                {
                    output_log
                        .read_to_string(&mut output)
                        .expect("failed to read from file");

                    output.contains("backend task executed")
                },
                "Backend task did not execute. OUTPUT: {}",
                output
            );

            assert!(
                output.contains("backend task executed"),
                "Backend task should execute"
            );
            assert!(
                !output.contains("frontend task executed"),
                "Frontend task should not execute on backend change. OUTPUT: {}",
                output
            );

            // Drain the backend phase fully (duplicate echoes, results
            // banner) before starting the regular phase.
            drain_until_quiet(&mut output_log, &mut output, 1_500);
            output.truncate(0);

            // Test 3: Trigger regular task (not in a group)
            write_to_file!(fixture.join("examples/workdir/regular/test.txt"));

            wait_until!(
                {
                    output_log
                        .read_to_string(&mut output)
                        .expect("failed to read from file");

                    output.contains("regular task executed")
                },
                "Regular task did not execute. OUTPUT: {}",
                output
            );

            assert!(
                output.contains("regular task executed"),
                "Regular task should execute"
            );
            assert!(
                !output.contains("frontend task executed"),
                "Frontend task should not execute on regular change. OUTPUT: {}",
                output
            );
            assert!(
                !output.contains("backend task executed"),
                "Backend task should not execute on regular change. OUTPUT: {}",
                output
            );
        },
    );
}

#[test]
#[cfg(feature = "test-integration")]
fn test_nested_groups_respect_ignore_patterns() {
    setup::with_example(
        setup::Options {
            output_file: "test_nested_groups_respect_ignore_patterns.log",
            example_file: "examples/test-nested-groups.yml",
        },
        |fzz_cmd, mut output_log, fixture| {
            // Create directories for test
            std::fs::create_dir_all(fixture.join("examples/workdir/frontend")).ok();

            let mut child = fzz_cmd.arg("-v").spawn().expect("failed to spawn process");
            let mut output = String::new();
            defer!({
                child.kill().expect("failed to close process");
            });

            wait_until!(
                {
                    output_log
                        .read_to_string(&mut output)
                        .expect("failed to read from file");

                    output.contains("Funzzy verbose") && output.contains("Watching...")
                },
                "Funzzy did not start. OUTPUT: {}",
                output
            );

            output.truncate(0);

            // Trigger a .log file in frontend (should be ignored by frontend group)
            write_to_file!(fixture.join("examples/workdir/frontend/test.log"));

            wait_until!(
                {
                    output_log
                        .read_to_string(&mut output)
                        .expect("failed to read from file");

                    output.contains("Funzzy verbose: Events Ok")
                        && output.contains("examples/workdir/frontend/test.log")
                },
                "Event not detected. OUTPUT: {}",
                output
            );

            // Give it a moment to potentially trigger (it shouldn't)
            std::thread::sleep(std::time::Duration::from_millis(500));

            output_log
                .read_to_string(&mut output)
                .expect("failed to read from file");

            assert!(
                !output.contains("frontend task executed"),
                ".log file should be ignored by frontend group. OUTPUT: {}",
                output
            );
        },
    );
}
