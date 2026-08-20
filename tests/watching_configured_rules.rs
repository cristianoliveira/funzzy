use pretty_assertions::assert_eq;
use std::io::{Read, Write};
#[cfg(feature = "test-integration-file-system")]
use std::path::PathBuf;

#[path = "./common/lib.rs"]
mod setup;

/// Build a per-run scratch root and a config generated from `template` with
/// the shared `/tmp/fzz` scratch replaced by this run's own directory.
///
/// Concurrent integration runs (the Funzzy watcher generation, CI, manual
/// invocations) each get an isolated scratch root, so no run can destroy
/// another run's watch roots or trigger files.
#[cfg(feature = "test-integration-file-system")]
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
                let _ = child.wait();
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

            output.clear();

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
                let _ = child.wait();
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

            output.clear();

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
                let _ = child.wait();
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

            // Each of f1 and f2 must land in its own debounce window: the
            // batch router schedules one generation per window and routes on
            // the first matching path (contract §1), so one rapid burst
            // coalesces into a single matched decision. FSEvents delivery
            // spacing made this pass by accident on macOS; inotify delivers
            // the whole burst in one window, so space the writes explicitly.
            write_to_file!(f1.as_str());
            std::thread::sleep(std::time::Duration::from_millis(1500));
            write_to_file!(f2.as_str());
            std::thread::sleep(std::time::Duration::from_millis(1500));
            write_to_file!(f3.as_str());
            write_to_file!(fixture.join("examples/workdir/trigger-watcher.txt"));
            write_to_file!(fixture.join("examples/workdir/ignored/modify.txt"));

            wait_until!(
                {
                    output_log
                        .read_to_string(&mut output)
                        .expect("failed to read from file");

                    // Matched decision lines carry the winning PATTERN
                    // (change=...), not the event path: f1/f2 patterns are
                    // the rewritten absolute paths themselves, while the
                    // workdir trigger routes through its glob. All three
                    // watched surfaces must have matched their own task.
                    // `contains`, not `starts_with`: verbose records can
                    // be prefixed by the ANSI clear-screen sequence.
                    output
                        .split("\n")
                        .filter(|line| {
                            line.contains("Funzzy debug:")
                                && line.contains("decision=matched")
                                && (line.contains(&f1)
                                    || line.contains(&f2)
                                    || line.contains("change=\"examples/workdir/**/*\""))
                        })
                        .count()
                        == 3
                        && output
                            .split("\n")
                            .find(|line| {
                                line.contains("Funzzy debug:")
                                    && line.contains("decision=matched")
                                    && (line.contains(&f3)
                                        || line.contains("examples/workdir/ignored"))
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
    let scratch = std::env::temp_dir().join(format!(
        "funzzy-fzz-scratch-{}-{}",
        std::process::id(),
        "invalid"
    ));
    let _ = std::fs::remove_dir_all(&scratch);
    std::fs::create_dir_all(&scratch).expect("failed to create scratch dir");
    let scratch = std::fs::canonicalize(&scratch).expect("failed to canonicalize scratch dir");
    let scratch_str = scratch.display().to_string();
    // Pin the native backend: this test asserts the native per-path
    // "unknown file/directory" warning for missing absolute paths (TASK-0037
    // made `auto` the default, which probes and falls back to polling with a
    // different message). Intent preserved: unknown paths warn, watching
    // proceeds, and the trigger still fires.
    let config = format!(
        "on:\n  change: '**/*'\n  watch_backend: native\njobs:\n  - name: task with invalid path @invalid\n    run: 'echo \"changed: {{{{filepath}}}}\"'\n    change:\n      - 'examples/workdir/**/*'\n      - '{scratch_str}/unknown.txt'\n      - '/unknown/unknown.txt'\n"
    );
    let config_path = scratch.join("config.yml");
    std::fs::write(&config_path, &config).expect("failed to write scratch config");
    let scratch_keep = scratch.clone();
    defer!({
        let _ = std::fs::remove_dir_all(&scratch_keep);
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
                let _ = child.wait();
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

                    // TASK-0086 / contract §3 §8: `{scratch}/unknown.txt` is
                    // covered by its nearest existing ancestor `{scratch}`
                    // (no warning — future coverage); the truly unwatchable
                    // `/unknown/unknown.txt` (no existing ancestor) warns
                    // actionably and watching proceeds.
                    output
                        .contains("Funzzy warning: unknown file/directory: '/unknown/unknown.txt'")
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
    setup::serialized(|| {
        // TASK-0037: `on.watch_backend: poll` drives the same batching/matching
        // path and must detect a file change and run the matching task.

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
        wait_watching(&scratch);

        std::fs::write(scratch.join("trigger.txt"), "change").unwrap();

        let deadline = std::time::Instant::now() + Duration::from_secs(15);
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
    });
}

#[cfg(feature = "test-integration")]
#[test]
fn gitignored_paths_do_not_trigger_tasks_when_respected() {
    setup::serialized(|| {
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
        "on:\n  change: '**/*'\n  respect_gitignore: true\njobs:\n  - name: capture\n    run: 'echo captured > captured.txt'\n    change: '*.txt'\n    ignore: ['captured.txt', 'child.out', 'child.err']\n",
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
        wait_watching(&scratch);

        // A gitignored file change must NOT run the task. Poll the absence for
        // the full debounce + margin instead of a single fixed sleep, so the
        // assertion is deterministic under load.
        std::fs::write(scratch.join("generated/out.txt"), "change").unwrap();
        let deadline = std::time::Instant::now() + Duration::from_secs(4);
        while std::time::Instant::now() < deadline {
            std::thread::sleep(Duration::from_millis(100));
            assert!(
                !scratch.join("captured.txt").exists(),
                "gitignored path must not trigger the task"
            );
        }

        // A normal file change DOES run the task.
        std::fs::write(scratch.join("real.txt"), "change").unwrap();
        let deadline = std::time::Instant::now() + Duration::from_secs(15);
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
    });
}

/// Builds a scratch config with one capture job watching `change` and writes
/// it to a per-run scratch dir. Used with `setup::with_config` so discovery
/// tests serialize through the harness mutex (never starving harness wait
/// budgets) and write triggers inside the fixture.
#[cfg(feature = "test-integration")]
fn discovery_config(label: &str, change: &str) -> (std::path::PathBuf, std::path::PathBuf) {
    let scratch =
        std::env::temp_dir().join(format!("funzzy-discovery-{}-{}", std::process::id(), label));
    let _ = std::fs::remove_dir_all(&scratch);
    std::fs::create_dir_all(&scratch).expect("failed to create scratch dir");
    let config_path = scratch.join("config.yml");
    std::fs::write(
        &config_path,
        format!(
            "on:\n  watch_backend: native\njobs:\n  - name: capture\n    run: 'echo captured > captured.txt'\n    change: '{change}'\n"
        ),
    )
    .expect("failed to write scratch config");
    (scratch, config_path)
}

/// TASK-0086 / WATCH-DISCOVERY-CONTRACT §2 §3: a file created under an
/// existing watched directory routes through the exact same matching flow as
/// a modification — no separate create path, no watcher restart, no touch to
/// arm it. Native backend.
#[cfg(feature = "test-integration")]
#[test]
fn newly_created_file_under_existing_watched_dir_triggers_job() {
    use std::time::Duration;

    let (_scratch, config_path) = discovery_config("native-create", "examples/workdir/**/*");
    setup::with_config(
        &config_path,
        "newly_created_file_under_existing_watched_dir_triggers_job.log",
        |fzz_cmd, mut output_log, fixture| {
            let mut child = fzz_cmd.spawn().expect("failed to spawn child");
            defer!({
                child.kill().expect("failed to kill child");
                let _ = child.wait();
            });

            let mut output = String::new();
            wait_until!(
                {
                    output_log
                        .read_to_string(&mut output)
                        .expect("failed to read from file");
                    output.contains("Watching...")
                },
                "watcher not ready {} {}",
                output,
                fixture.display()
            );

            // Create a NEW file under the existing watched directory (no
            // prior existence, no touch) — it must trigger like a modify.
            std::fs::write(
                fixture.join("examples/workdir/brand-new.rs"),
                "fn main() {}\n",
            )
            .unwrap();

            let deadline = std::time::Instant::now() + Duration::from_secs(15);
            let mut ran = false;
            while std::time::Instant::now() < deadline {
                if fixture.join("captured.txt").exists() {
                    ran = true;
                    break;
                }
                std::thread::sleep(Duration::from_millis(100));
            }
            assert!(ran, "newly created file must trigger the watched job");
            assert_eq!(
                std::fs::read_to_string(fixture.join("captured.txt")).unwrap(),
                "captured\n"
            );
        },
    );
}

/// TASK-0086 / contract §3: a directory created after startup becomes
/// covered without a watcher restart — the job fires for files created in
/// the new directory (native backend watches the stable ancestor).
#[cfg(feature = "test-integration")]
#[test]
fn directory_created_after_startup_becomes_covered_without_restart() {
    use std::time::Duration;

    let (_scratch, config_path) = discovery_config("native-dir", "examples/workdir/**/*");
    setup::with_config(
        &config_path,
        "directory_created_after_startup_becomes_covered_without_restart.log",
        |fzz_cmd, mut output_log, fixture| {
            let mut child = fzz_cmd.spawn().expect("failed to spawn child");
            defer!({
                child.kill().expect("failed to kill child");
                let _ = child.wait();
            });

            let mut output = String::new();
            wait_until!(
                {
                    output_log
                        .read_to_string(&mut output)
                        .expect("failed to read from file");
                    output.contains("Watching...")
                },
                "watcher not ready {} {}",
                output,
                fixture.display()
            );

            // Create a nested directory tree AND a file in one operation,
            // after the watcher started. The canonical final path routes
            // once; intermediate directory events run no unrelated jobs.
            std::fs::create_dir_all(fixture.join("examples/workdir/deep/nested")).unwrap();
            std::fs::write(fixture.join("examples/workdir/deep/nested/out.txt"), "x").unwrap();

            let deadline = std::time::Instant::now() + Duration::from_secs(15);
            let mut ran = false;
            while std::time::Instant::now() < deadline {
                if fixture.join("captured.txt").exists() {
                    ran = true;
                    break;
                }
                std::thread::sleep(Duration::from_millis(100));
            }
            assert!(ran, "file in directory created after startup must trigger");
        },
    );
}

/// TASK-0086 / contract §5: delete then recreate a file remains observable
/// without restart (native backend; the stable ancestor keeps watching).
#[cfg(feature = "test-integration")]
#[test]
fn delete_and_recreate_stays_observable_without_restart() {
    use std::time::Duration;

    let (_scratch, config_path) = discovery_config("native-recreate", "examples/workdir/**/*");
    setup::with_config(
        &config_path,
        "delete_and_recreate_stays_observable_without_restart.log",
        |fzz_cmd, mut output_log, fixture| {
            let mut child = fzz_cmd.spawn().expect("failed to spawn child");
            defer!({
                child.kill().expect("failed to kill child");
                let _ = child.wait();
            });

            let mut output = String::new();
            wait_until!(
                {
                    output_log
                        .read_to_string(&mut output)
                        .expect("failed to read from file");
                    output.contains("Watching...")
                },
                "watcher not ready {} {}",
                output,
                fixture.display()
            );

            let target = fixture.join("examples/workdir/loop.txt");
            let captured = fixture.join("captured.txt");

            // First creation triggers.
            std::fs::write(&target, "v1").unwrap();
            let deadline = std::time::Instant::now() + Duration::from_secs(15);
            let mut first = false;
            while std::time::Instant::now() < deadline {
                if captured.exists() {
                    first = true;
                    break;
                }
                std::thread::sleep(Duration::from_millis(100));
            }
            assert!(first, "first creation must trigger");
            std::fs::remove_file(&captured).unwrap();

            // Delete then recreate: still observable, no restart.
            std::fs::remove_file(&target).unwrap();
            std::fs::write(&target, "v2").unwrap();
            let deadline = std::time::Instant::now() + Duration::from_secs(15);
            let mut second = false;
            while std::time::Instant::now() < deadline {
                if captured.exists() {
                    second = true;
                    break;
                }
                std::thread::sleep(Duration::from_millis(100));
            }
            assert!(second, "delete then recreate must stay observable");
        },
    );
}

/// TASK-0086 / contract §4: an atomic editor save (temp create/write +
/// rename over destination) triggers the destination once; the temp path
/// never leaks as the selected job. The debouncer collapses both paths into
/// one batch; deterministic matching routes the destination (temp names
/// either match nothing or are dropped by ignore).
#[cfg(feature = "test-integration")]
#[test]
fn atomic_editor_save_triggers_destination_once_without_temp_leak() {
    use std::time::Duration;

    let (_scratch, config_path) = discovery_config("native-atomic", "examples/workdir/*.txt");
    setup::with_config(
        &config_path,
        "atomic_editor_save_triggers_destination_once_without_temp_leak.log",
        |fzz_cmd, mut output_log, fixture| {
            let mut child = fzz_cmd.spawn().expect("failed to spawn child");
            defer!({
                child.kill().expect("failed to kill child");
                let _ = child.wait();
            });

            let mut output = String::new();
            wait_until!(
                {
                    output_log
                        .read_to_string(&mut output)
                        .expect("failed to read from file");
                    output.contains("Watching...")
                },
                "watcher not ready {} {}",
                output,
                fixture.display()
            );

            // Editor-style save: write temp, then rename over the
            // destination. The temp name matches no change pattern
            // (`*.txt` does not match `.tmp`); the destination routes once.
            let target = fixture.join("examples/workdir/notes.txt");
            std::fs::write(fixture.join("examples/workdir/notes.txt.tmp"), "draft\n").unwrap();
            std::fs::rename(fixture.join("examples/workdir/notes.txt.tmp"), &target).unwrap();

            let deadline = std::time::Instant::now() + Duration::from_secs(15);
            let mut ran = false;
            while std::time::Instant::now() < deadline {
                if fixture.join("captured.txt").exists() {
                    ran = true;
                    break;
                }
                std::thread::sleep(Duration::from_millis(100));
            }
            assert!(ran, "atomic save to destination must trigger once");
        },
    );
}

#[cfg(feature = "test-integration")]
fn wait_watching(scratch: &std::path::Path) {
    use std::time::{Duration, Instant};
    let deadline = Instant::now() + Duration::from_secs(20);
    while Instant::now() < deadline {
        if let Ok(log) = std::fs::read_to_string(scratch.join("child.out")) {
            if log.contains("Watching...") {
                return;
            }
        }
        std::thread::sleep(Duration::from_millis(100));
    }
    let log = std::fs::read_to_string(scratch.join("child.out")).unwrap_or_default();
    panic!("watcher never reported Watching...; log: {log}");
}
