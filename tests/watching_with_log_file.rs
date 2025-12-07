use std::io::prelude::*;
use std::process::Child;

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

#[test]
fn test_it_truncates_log_on_config_change() {
    struct Cleanup {
        log_path: std::path::PathBuf,
        config_path: std::path::PathBuf,
        original_config: String,
        child: Option<Child>,
    }

    impl Cleanup {
        fn new(
            log_path: std::path::PathBuf,
            config_path: std::path::PathBuf,
            original_config: String,
            child: Child,
        ) -> Self {
            Cleanup {
                log_path,
                config_path,
                original_config,
                child: Some(child),
            }
        }
    }

    impl Drop for Cleanup {
        fn drop(&mut self) {
            if let Some(mut child) = self.child.take() {
                let _ = child.kill();
            }
            let _ = std::fs::write(&self.config_path, &self.original_config);
            let _ = std::fs::remove_file(&self.log_path);
        }
    }

    setup::with_example(
        setup::Options {
            output_file: "test_it_truncates_log_on_config_change.console.log",
            example_file: "examples/simple-case.yml",
        },
        |fzz_cmd, mut output_log| {
            let log_path = std::env::temp_dir().join("funzzy_truncate_on_change.log");
            let _ = std::fs::remove_file(&log_path);

            let config_path = std::env::current_dir()
                .expect("failed to get current directory")
                .join("examples/simple-case.yml");
            let original_config =
                std::fs::read_to_string(&config_path).expect("failed to read config for backup");

            let child = fzz_cmd
                .arg("--log-file")
                .arg(&log_path)
                .arg("--log-truncate-on-change")
                .spawn()
                .expect("failed to spawn child");

            let _cleanup = Cleanup::new(
                log_path.clone(),
                config_path.clone(),
                original_config.clone(),
                child,
            );

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
                    match std::fs::read_to_string(&log_path) {
                        Ok(contents) => contents.contains("something changed in workdir!"),
                        Err(_) => false,
                    }
                },
                "Failed to observe watcher output in log file"
            );

            let mutated_config = format!("{}\n# temp change\n", original_config);
            std::fs::write(&config_path, mutated_config).expect("failed to mutate config file");

            let mut final_contents = None;
            for _ in 0..50 {
                if let Ok(contents) = std::fs::read_to_string(&log_path) {
                    if contents
                        .contains("Funzzy: Log file truncated before reloading configuration.")
                    {
                        final_contents = Some(contents);
                        break;
                    }
                }
                std::thread::sleep(std::time::Duration::from_millis(100));
            }

            let final_contents = final_contents.expect("truncate message not found in log");

            let truncate_pos = final_contents
                .find("Funzzy: Log file truncated before reloading configuration.")
                .expect("truncate message not found in log");
            let task_pos = final_contents.find("something changed in workdir!");

            if let Some(pos) = task_pos {
                assert!(
                    pos >= truncate_pos,
                    "Task output appears before truncation marker; expected truncation. Contents: {}",
                    final_contents
                );
            }
        },
    );
}
