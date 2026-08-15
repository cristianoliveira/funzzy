use std::io::prelude::*;

#[path = "./common/lib.rs"]
mod setup;

/// TASK-0088/0090: a VALID config change no longer self-SIGTERMs. The
/// watcher stays alive in the same process and hot-reloads (the reload
/// example is valid — its init tasks touch the config file). PID continuity
/// is the proof: the child process never exits on a valid save.
#[test]
fn valid_config_change_hot_reloads_without_process_exit() {
    setup::with_example(
        setup::Options {
            output_file: "valid_config_change_hot_reloads_without_process_exit.log",
            example_file: "examples/reload-config-example.yml",
        },
        |fzz_cmd, mut output_log, _fixture| {
            let mut child = fzz_cmd
                .arg("--restart")
                .spawn()
                .expect("failed to spawn child");

            let mut output = String::new();
            wait_until!(
                {
                    output_log
                        .read_to_string(&mut output)
                        .expect("failed to read from file");

                    output.contains("Config change is valid; hot-reloading")
                },
                "valid config change must hot-reload {}",
                output
            );

            // The watcher must still be running (same process, no SIGTERM).
            let exited = child.try_wait().expect("try_wait").is_some();
            assert!(
                !exited,
                "valid reload must NOT terminate the watcher process"
            );
            assert!(!output.contains("Funzzy warning: unknown file/directory"));

            let _ = child.kill();
            let _ = child.wait();
        },
    );
}
