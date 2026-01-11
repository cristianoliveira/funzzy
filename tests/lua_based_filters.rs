use std::io::prelude::*;

#[path = "./common/lib.rs"]
mod setup;

#[test]
fn test_it_loads_lua_based_filter_config() {
    // Test that the config file with Lua pattern loads without error
    // This doesn't test actual Lua evaluation, just parsing
    setup::with_example(
        setup::Options {
            output_file: "test_it_loads_lua_based_filter_config.log",
            example_file: "examples/lua-based-filter.yml",
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

                    output.contains("Watching...")
                },
                "Funzzy failed to start with Lua config: {}",
                output
            );

            // The watcher started successfully
            // Note: Lua evaluation is not yet implemented, so the task won't run
            // This test just verifies the config parses and watcher starts
        },
    );
}

#[test]
fn test_it_runs_task_when_lua_predicate_matches() {
    // This test documents the expected behavior when Lua evaluation is implemented
    // It will run the task when a file containing "trigger" in its path changes
    setup::with_example(
        setup::Options {
            output_file: "test_it_runs_task_when_lua_predicate_matches.log",
            example_file: "examples/lua-based-filter.yml",
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

                    output.contains("Watching...")
                },
                "Funzzy failed to start: {}",
                output
            );

            // Write a file that contains "trigger" in its name
            write_to_file!("examples/workdir/trigger-lua-test.txt");

            wait_until!(
                {
                    output_log
                        .read_to_string(&mut output)
                        .expect("failed to read from file");

                    output.contains("File contains 'trigger' in path")
                },
                "Task should have run for file containing 'trigger': {}",
                output
            );

            // Write a file that does NOT contain "trigger" in its name
            write_to_file!("examples/workdir/other-file.txt");

            // Give it a moment, then check task didn't run
            std::thread::sleep(std::time::Duration::from_millis(1500));
            use std::io::Seek;
            output_log.seek(std::io::SeekFrom::Start(0)).unwrap();
            output.clear();
            output_log
                .read_to_string(&mut output)
                .expect("failed to read from file");
            let task_run_count = output
                .lines()
                .filter(|line| line.trim() == "File contains 'trigger' in path")
                .count();
            assert_eq!(
                task_run_count, 1,
                "Task should have run only once (for trigger file), but ran {} times. Output: {}",
                task_run_count, output
            );
        },
    );
}
