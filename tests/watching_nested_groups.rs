use pretty_assertions::assert_eq;
use std::io::prelude::*;

#[path = "./common/lib.rs"]
mod setup;

#[test]
#[cfg(feature = "test-integration")]
fn test_nested_groups_watch_different_patterns() {
    setup::with_example(
        setup::Options {
            output_file: "test_nested_groups_watch_different_patterns.log",
            example_file: "examples/test-nested-groups.yml",
        },
        |fzz_cmd, mut output_log| {
            // Create directories for test
            std::fs::create_dir_all("examples/workdir/frontend").ok();
            std::fs::create_dir_all("examples/workdir/backend").ok();
            std::fs::create_dir_all("examples/workdir/regular").ok();

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

            output.truncate(0);

            // Test 1: Trigger frontend group
            write_to_file!("examples/workdir/frontend/test.js");

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

            output.truncate(0);

            // Test 2: Trigger backend group
            write_to_file!("examples/workdir/backend/test.rs");

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

            output.truncate(0);

            // Test 3: Trigger regular task (not in a group)
            write_to_file!("examples/workdir/regular/test.txt");

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
        |fzz_cmd, mut output_log| {
            // Create directories for test
            std::fs::create_dir_all("examples/workdir/frontend").ok();

            let mut child = fzz_cmd.arg("-V").spawn().expect("failed to spawn process");
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
            write_to_file!("examples/workdir/frontend/test.log");

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
