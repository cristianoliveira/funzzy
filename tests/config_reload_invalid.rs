//! TASK-0088/0090: invalid config reloads are fatal with nonzero exit and a
//! terminal error — never a silent stale continuation, never an abrupt
//! self-SIGTERM that skips cleanup.

use std::time::Duration;

#[path = "./common/lib.rs"]
mod setup;

fn scratch_root(label: &str) -> std::path::PathBuf {
    let scratch = std::env::temp_dir().join(format!(
        "funzzy-invalid-reload-{}-{}",
        std::process::id(),
        label
    ));
    let _ = std::fs::remove_dir_all(&scratch);
    std::fs::create_dir_all(&scratch).unwrap();
    std::fs::create_dir_all(scratch.join("src")).unwrap();
    scratch
}

/// An invalid replacement (broken YAML) must terminate the watcher with a
/// nonzero exit and a visible terminal error, after a bounded validation
/// window — never keep running stale config silently.
#[test]
fn invalid_config_replacement_is_fatal_with_nonzero_exit() {
    setup::serialized(|| {
        let scratch = scratch_root("broken-yaml");
        std::fs::write(
            scratch.join(".watch.yaml"),
            "jobs:\n  - name: build\n    run: echo hi\n    change: 'src/**'\n",
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

        // Wait for the watcher to be up.
        let deadline = std::time::Instant::now() + Duration::from_secs(20);
        loop {
            if let Ok(log) = std::fs::read_to_string(scratch.join("child.out")) {
                if log.contains("Watching...") {
                    break;
                }
            }
            assert!(
                std::time::Instant::now() < deadline,
                "watcher never became ready"
            );
            std::thread::sleep(Duration::from_millis(100));
        }

        // Replace the config with invalid YAML.
        std::fs::write(scratch.join(".watch.yaml"), "jobs: [unclosed").unwrap();

        // The watcher must exit nonzero with a terminal config error.
        let deadline = std::time::Instant::now() + Duration::from_secs(20);
        let exit = loop {
            if let Some(status) = child.try_wait().expect("try_wait") {
                break status;
            }
            assert!(
                std::time::Instant::now() < deadline,
                "invalid config must terminate the watcher"
            );
            std::thread::sleep(Duration::from_millis(100));
        };
        let _ = child.wait();
        assert!(
            !exit.success(),
            "invalid config must exit nonzero, got {exit:?}"
        );

        let log = std::fs::read_to_string(scratch.join("child.out")).unwrap_or_default();
        assert!(
            log.contains("Fatal configuration error"),
            "terminal config error must be visible: {log}"
        );
        assert!(
            log.contains("invalid config"),
            "the gate and reason must be named: {log}"
        );
        std::fs::remove_dir_all(&scratch).unwrap();
    });
}

/// TASK-0090: a valid config that adds a NEW watch root hot-reloads and the
/// new root immediately observes files — prepare→commit→retire with a live
/// backend root swap (contract §4). The process never exits.
#[test]
fn valid_reload_adds_new_root_and_observes_files_after_commit() {
    setup::serialized(|| {
        use std::time::Duration;

        let scratch = scratch_root("add-root");
        // The new root directory exists BEFORE the reload, so the swap's
        // watch registration targets an existing path (native notify cannot
        // watch a nonexistent directory). Events created after the commit
        // boundary are observed by the new revision.
        std::fs::create_dir_all(scratch.join("docs")).unwrap();
        std::fs::write(
            scratch.join(".watch.yaml"),
            "on:\n  socket: sock\njobs:\n  - name: capture\n    run: 'echo captured > verdict.txt'\n    change: 'src/**'\n",
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

        // Wait for the control socket (the watcher is up and subscribing).
        let deadline = std::time::Instant::now() + Duration::from_secs(20);
        let socket = scratch.join("sock");
        loop {
            if std::os::unix::net::UnixStream::connect(&socket).is_ok() {
                break;
            }
            assert!(
                std::time::Instant::now() < deadline,
                "control socket never connectable"
            );
            std::thread::sleep(Duration::from_millis(100));
        }

        // Valid reload: add a `docs/**` job (new root). Process must survive.
        std::fs::write(
            scratch.join(".watch.yaml"),
            "on:\n  socket: sock\njobs:\n  - name: capture\n    run: 'echo captured > verdict.txt'\n    change: 'src/**'\n  - name: docs\n    run: 'echo docs > docs-verdict.txt'\n    change: 'docs/**'\n",
        )
        .unwrap();

        // Wait for the hot-reload message.
        let deadline = std::time::Instant::now() + Duration::from_secs(20);
        loop {
            if let Ok(log) = std::fs::read_to_string(scratch.join("child.out")) {
                if log.contains("hot-reloading to revision 2") {
                    break;
                }
            }
            assert!(
                std::time::Instant::now() < deadline,
                "hot reload never reported"
            );
            std::thread::sleep(Duration::from_millis(100));
        }
        assert!(
            child.try_wait().expect("try_wait").is_none(),
            "valid reload must not exit the process"
        );

        // Create a file in the NEW root AFTER the commit boundary: the docs
        // job must run.
        std::fs::write(scratch.join("docs/guide.md"), "content").unwrap();
        let deadline = std::time::Instant::now() + Duration::from_secs(15);
        let mut ran = false;
        while std::time::Instant::now() < deadline {
            if scratch.join("docs-verdict.txt").exists() {
                ran = true;
                break;
            }
            std::thread::sleep(Duration::from_millis(100));
        }
        if !ran {
            let log = std::fs::read_to_string(scratch.join("child.out")).unwrap_or_default();
            let err = std::fs::read_to_string(scratch.join("child.err")).unwrap_or_default();
            eprintln!("=== child log ===\n{log}\n=== child err ===\n{err}");
        }
        let _ = child.kill();
        let _ = child.wait();
        assert!(ran, "new root must observe files after reload");
        std::fs::remove_dir_all(&scratch).unwrap();
    });
}
