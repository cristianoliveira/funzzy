use pretty_assertions::assert_eq;
use std::io::prelude::*;
use std::path::PathBuf;

#[path = "./common/lib.rs"]
mod setup;

/// Build a per-run scratch root and a config generated from `template` with
/// the shared `/tmp/fzz` scratch replaced by this run's own directory.
///
/// Concurrent integration runs (the Funzzy watcher generation, CI, manual
/// invocations) each get an isolated scratch root, so no run can destroy
/// another run's watch roots or trigger files.
fn scratch_config(template: &str, label: &str) -> (PathBuf, PathBuf, String) {
    let scratch = std::env::temp_dir().join(format!(
        "funzzy-fzz-scratch-{}-{}",
        std::process::id(),
        label
    ));
    let _ = std::fs::remove_dir_all(&scratch);
    std::fs::create_dir_all(&scratch).expect("failed to create scratch dir");

    // Resolve symlink prefixes (macOS maps /var -> /private/var) so the
    // config paths, the files the test writes, and the paths notify reports
    // are the same canonical strings; otherwise events never match patterns.
    let scratch = std::fs::canonicalize(&scratch).expect("failed to canonicalize scratch dir");

    let template_content =
        std::fs::read_to_string(template).expect("failed to read example template");
    let scratch_str = scratch.display().to_string();
    let config = template_content.replace("/tmp/fzz", &scratch_str);
    let config_path = scratch.join("config.yml");
    std::fs::write(&config_path, config).expect("failed to write scratch config");

    (scratch, config_path, scratch_str)
}

#[test]
fn test_it_is_not_triggered_by_ignored_files() {
    setup::with_example(
        setup::Options {
            output_file: "test_it_is_not_triggered_by_ignored_files.log",
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

            write_to_file!(fixture.join("examples/workdir/another_ignored_file.foo"));

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
        },
    );
}

#[test]
fn test_it_watch_files_and_execute_configured_commands() {
    setup::with_example(
        setup::Options {
            example_file: "examples/simple-case.yml",
            output_file: "test_it_watch_files_and_execute_configured_commands.log",
        },
        |fzz_cmd, mut output_log, fixture| {
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
                "OUTPUT: {}",
                output
            );

            output.truncate(0);

            write_to_file!(fixture.join("examples/workdir/trigger-watcher.txt"));

            wait_until!(
                {
                    output_log
                        .read_to_string(&mut output)
                        .expect("failed to read from file");

                    output.contains("Funzzy results")
                },
                "OUTPUT: {}",
                output
            );

            assert_eq!(
                setup::clean_output(&output),
                "
[2J
Funzzy: echo first 

first

Funzzy: echo second 

second

Funzzy: echo complex | sed s/complex/third/g 

third

Funzzy: echo 'something changed in workdir!' 

something changed in workdir!
Funzzy results ----------------------------
Success; Completed: 4; Failed: 0; Duration: 0.0000s"
            );
        },
    );
}

#[test]
#[cfg(feature = "test-integration-file-system")]
fn accepts_full_or_relativepaths() {
    // Each run gets its own scratch root so concurrent integration runs
    // (watcher generation, CI, manual) never clobber each other's watch
    // roots or trigger files.
    let (scratch, config_path, scratch_str) =
        scratch_config("examples/tasks-with-absolute-paths.yml", "valid");
    defer!({
        let _ = std::fs::remove_dir_all(&scratch);
    });

    let f1 = format!("{}/accepts_full_or_relativepaths.txt", scratch_str);
    let f2 = format!("{}/accepts_full_or_relativepaths2.txt", scratch_str);
    let f3 = format!("{}/accepts_full_or_relativepaths3.txt", scratch_str);

    setup::with_config(
        &config_path,
        "accepts_full_or_relativepaths.log",
        |fzz_cmd, mut output_log, fixture| {
            // Initialize the files
            write_to_file!(f1.as_str());
            write_to_file!(f2.as_str());
            write_to_file!(f3.as_str());

            let mut child = fzz_cmd
                .args(["watch", "@valid"])
                .arg("-v")
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

                    output.contains("decision=startup") && output.contains("Watching...")
                },
                "fzz not started in verbose {}",
                output
            );

            write_to_file!(f1.as_str());
            write_to_file!(f2.as_str());
            write_to_file!(f3.as_str());
            write_to_file!(fixture.join("examples/workdir/trigger-watcher.txt"));
            write_to_file!(fixture.join("examples/workdir/ignored/modify.txt"));

            wait_until!(
                {
                    output_log
                        .read_to_string(&mut output)
                        .expect("failed to read from file");

                    output
                        .split("\n")
                        .filter(|line| {
                            line.starts_with("Funzzy debug:")
                                && line.contains("decision=matched")
                                && (line.contains(&f1)
                                    || line.contains(&f2)
                                    || line.contains("examples/workdir/trigger-watcher.txt"))
                        })
                        .count()
                        == 3
                        && output
                            .split("\n")
                            .find(|line| {
                                line.starts_with("Funzzy debug:")
                                    && line.contains("decision=matched")
                                    && (line.contains(&f3)
                                        || line.contains("examples/workdir/ignored/modify.txt"))
                            })
                            .is_none()
                },
                "triggered task that was not in watch list {}",
                output
            );

            let _ = scratch;
        },
    );
}

#[test]
fn fails_with_unkown_paths() {
    let (scratch, config_path, scratch_str) =
        scratch_config("examples/tasks-with-absolute-paths.yml", "invalid");
    defer!({
        let _ = std::fs::remove_dir_all(&scratch);
    });

    setup::with_config(
        &config_path,
        "fails_with_unkown_paths.log",
        |fzz_cmd, mut output_log, fixture| {
            let mut child = fzz_cmd
                .args(["watch", "@invalid"])
                .arg("-v")
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

                    output.contains("decision=startup")
                },
                "fzz not started in verbose {}",
                output
            );

            wait_until!(
                {
                    output_log
                        .read_to_string(&mut output)
                        .expect("failed to read from file");

                    output.contains(&format!(
                        "Funzzy warning: unknown file/directory: '{}/unknown.txt'",
                        scratch_str
                    ))
                },
                "expected output contain error explanation but got {}",
                output
            );

            write_to_file!(fixture.join("examples/workdir/trigger-watcher.txt"));
            wait_until!(
                {
                    output_log
                        .read_to_string(&mut output)
                        .expect("failed to read from file");

                    output.contains("examples/workdir/trigger-watcher.txt")
                        && output.contains("Funzzy results")
                },
                "expected output contain error explanation but got {}",
                output
            );
        },
    );
}

#[cfg(feature = "test-integration")]
#[test]
fn poll_backend_detects_changes_and_runs_tasks() {
    // TASK-0037: `on.watch_backend: poll` drives the same batching/matching
    // path and must detect a file change and run the matching task.
    use std::io::prelude::*;
    use std::time::Duration;

    let scratch = std::env::temp_dir().join(format!(
        "funzzy-poll-watch-{}-{}",
        std::process::id(),
        "poll-backend"
    ));
    let _ = std::fs::remove_dir_all(&scratch);
    std::fs::create_dir_all(&scratch).unwrap();
    std::fs::write(
        scratch.join(".watch.yaml"),
        "on:\n  change: '**/*'\n  watch_backend: poll\n  poll_interval: 100ms\njobs:\n  - name: capture\n    run: 'echo captured > captured.txt'\n    change: '*.txt'\n    ignore: 'captured.txt'\n",
    )
    .unwrap();

    let output_log = std::fs::File::create(scratch.join("child.out")).unwrap();
    let mut child = std::process::Command::new(env!("CARGO_BIN_EXE_fzz"))
        .current_dir(&scratch)
        .env("FUNZZY_COLORED", "false")
        .stdout(std::process::Stdio::from(output_log))
        .stderr(std::process::Stdio::null())
        .spawn()
        .unwrap();

    // Wait for the watcher to be up (poll backend has no socket; wait for
    // the watching line via a small poll on the process output).
    std::thread::sleep(Duration::from_millis(500));

    std::fs::write(scratch.join("trigger.txt"), "change").unwrap();

    let mut deadline = std::time::Instant::now() + Duration::from_secs(15);
    let mut ran = false;
    while std::time::Instant::now() < deadline {
        if scratch.join("captured.txt").exists() {
            ran = true;
            break;
        }
        std::thread::sleep(Duration::from_millis(100));
    }
    let _ = child.kill();
    let _ = child.wait();
    assert!(ran, "poll backend must detect the change and run the task");
    assert_eq!(
        std::fs::read_to_string(scratch.join("captured.txt")).unwrap(),
        "captured\n"
    );
    std::fs::remove_dir_all(&scratch).unwrap();
}

#[cfg(feature = "test-integration")]
#[test]
fn gitignored_paths_do_not_trigger_tasks_when_respected() {
    // TASK-0036: `on.respect_gitignore: true` excludes paths matched by
    // `.gitignore` from triggering tasks; explicit config `ignore` still wins
    // and works without the knob.
    use std::time::Duration;

    let scratch = std::env::temp_dir().join(format!(
        "funzzy-gitignore-watch-{}-{}",
        std::process::id(),
        "gitignore-backend"
    ));
    let _ = std::fs::remove_dir_all(&scratch);
    std::fs::create_dir_all(&scratch).unwrap();
    std::fs::write(scratch.join(".gitignore"), "generated/\n").unwrap();
    std::fs::create_dir_all(scratch.join("generated")).unwrap();
    std::fs::write(
        scratch.join(".watch.yaml"),
        "on:\n  change: '**/*'\n  respect_gitignore: true\njobs:\n  - name: capture\n    run: 'echo captured > captured.txt'\n    change: '*.txt'\n    ignore: 'captured.txt'\n",
    )
    .unwrap();

    let output_log = std::fs::File::create(scratch.join("child.out")).unwrap();
    let error_log = std::fs::File::create(scratch.join("child.err")).unwrap();
    let mut child = std::process::Command::new(env!("CARGO_BIN_EXE_fzz"))
        .current_dir(&scratch)
        .env("FUNZZY_COLORED", "false")
        .stdout(std::process::Stdio::from(output_log))
        .stderr(std::process::Stdio::from(error_log))
        .spawn()
        .unwrap();
    std::thread::sleep(Duration::from_millis(600));

    // A gitignored file change must NOT run the task.
    std::fs::write(scratch.join("generated/out.txt"), "change").unwrap();
    std::thread::sleep(Duration::from_millis(1500));
    assert!(
        !scratch.join("captured.txt").exists(),
        "gitignored path must not trigger the task"
    );

    // A normal file change DOES run the task.
    std::fs::write(scratch.join("real.txt"), "change").unwrap();
    let mut deadline = std::time::Instant::now() + Duration::from_secs(15);
    let mut ran = false;
    while std::time::Instant::now() < deadline {
        if scratch.join("captured.txt").exists() {
            ran = true;
            break;
        }
        std::thread::sleep(Duration::from_millis(100));
    }
    let _ = child.kill();
    let _ = child.wait();
    if !ran {
        let log = std::fs::read_to_string(scratch.join("child.out")).unwrap_or_default();
        let err = std::fs::read_to_string(scratch.join("child.err")).unwrap_or_default();
        eprintln!("watcher log: {log}");
        eprintln!("watcher err: {err}");
    }
    assert!(ran, "non-gitignored change must trigger the task");
    std::fs::remove_dir_all(&scratch).unwrap();
}
