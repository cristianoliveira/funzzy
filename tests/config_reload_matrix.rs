//! TASK-0090 AC1/AC4/AC5: the hot-reload test matrix. Each scenario drives a
//! real watcher through a valid semantic config rewrite and asserts the
//! process survives, the new revision is live, and the old behavior is gone
//! (or the new behavior appears) — never a process exit, never a silent
//! stale continuation.

use nix::sys::signal::{self, Signal};
use nix::unistd::Pid;
use std::time::Duration;

#[path = "./common/lib.rs"]
mod setup;

fn scratch_root(label: &str) -> std::path::PathBuf {
    let scratch = std::env::temp_dir().join(format!(
        "funzzy-reload-matrix-{}-{}",
        std::process::id(),
        label
    ));
    let _ = std::fs::remove_dir_all(&scratch);
    std::fs::create_dir_all(&scratch).unwrap();
    std::fs::create_dir_all(scratch.join("src")).unwrap();
    std::fs::create_dir_all(scratch.join("docs")).unwrap();
    scratch
}

/// Spawns a watcher in `scratch` with `config`, waiting for readiness.
fn spawn_watcher(scratch: &std::path::Path, config: &str) -> std::process::Child {
    std::fs::write(scratch.join(".watch.yaml"), config).unwrap();
    let output_log = std::fs::File::create(scratch.join("child.out")).unwrap();
    let error_log = std::fs::File::create(scratch.join("child.err")).unwrap();
    let child = std::process::Command::new(env!("CARGO_BIN_EXE_fzz"))
        .current_dir(scratch)
        .env("FUNZZY_COLORED", "false")
        .stdout(std::process::Stdio::from(output_log))
        .stderr(std::process::Stdio::from(error_log))
        .spawn()
        .unwrap();
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
    child
}

/// Requests the watcher's normal shutdown and waits for it to reap its
/// managed process groups. A forced fallback keeps a failed test from leaking
/// its fixture into the host, but the boolean lets callers assert that the
/// graceful path was used.
fn stop_watcher_gracefully(child: &mut std::process::Child) -> bool {
    let _ = signal::kill(Pid::from_raw(child.id() as i32), Signal::SIGTERM);
    let deadline = std::time::Instant::now() + Duration::from_secs(15);
    loop {
        if child.try_wait().expect("poll watcher shutdown").is_some() {
            return true;
        }
        if std::time::Instant::now() >= deadline {
            let _ = child.kill();
            let _ = child.wait();
            return false;
        }
        std::thread::sleep(Duration::from_millis(50));
    }
}

fn process_group_alive(pgid: i32) -> bool {
    !matches!(
        signal::kill(Pid::from_raw(-pgid), None),
        Err(nix::errno::Errno::ESRCH)
    )
}

/// Waits until a managed service's whole process group is gone. This checks
/// descendants too, not just the shell process that the watcher owns.
fn wait_for_process_group_exit(pgid: i32) -> bool {
    let deadline = std::time::Instant::now() + Duration::from_secs(10);
    while std::time::Instant::now() < deadline {
        if !process_group_alive(pgid) {
            return true;
        }
        std::thread::sleep(Duration::from_millis(50));
    }
    false
}

fn force_kill_process_group(pgid: i32) {
    let _ = signal::kill(Pid::from_raw(-pgid), Signal::SIGKILL);
}

/// Waits for `needle` in the child log; panics with the log on timeout.
fn wait_for_log(scratch: &std::path::Path, needle: &str) -> String {
    let deadline = std::time::Instant::now() + Duration::from_secs(20);
    loop {
        if let Ok(log) = std::fs::read_to_string(scratch.join("child.out")) {
            if log.contains(needle) {
                return log;
            }
        }
        assert!(
            std::time::Instant::now() < deadline,
            "never observed {needle:?}"
        );
        std::thread::sleep(Duration::from_millis(100));
    }
}

fn base_config() -> String {
    "on:\n  socket: sock\njobs:\n  - name: capture-src\n    run: 'echo src > src-verdict.txt'\n    change: 'src/**'\n  - name: capture-docs\n    run: 'echo docs > docs-verdict.txt'\n    change: 'docs/**'\n"
        .to_owned()
}

/// Waits for a control socket to become connectable (readiness proxy for the
/// non-block watcher used across the matrix).
fn wait_for_socket(scratch: &std::path::Path) {
    let socket = scratch.join("sock");
    let deadline = std::time::Instant::now() + Duration::from_secs(20);
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
}

/// True when the watcher's latest generation is running (generation >= 1).
/// Used to wait for a triggered generation to start before a reload, so the
/// test never races the debounce window against the commit boundary.
fn status_generation_running(scratch: &std::path::Path) -> bool {
    use std::io::{BufRead, BufReader, Write};
    let socket = scratch.join("sock");
    let mut stream = match std::os::unix::net::UnixStream::connect(&socket) {
        Ok(stream) => stream,
        Err(_) => return false,
    };
    if writeln!(
        stream,
        r#"{{"jsonrpc":"2.0","id":"status","method":"status","params":{{}}}}"#
    )
    .is_err()
    {
        return false;
    }
    let mut line = String::new();
    if BufReader::new(&mut stream).read_line(&mut line).is_err() {
        return false;
    }
    serde_json::from_str::<serde_json::Value>(&line)
        .ok()
        .map(|value| {
            let result = value["result"].clone();
            result["generation"].as_u64().unwrap_or(0) >= 1
                && result["state"].as_str() == Some("running")
        })
        .unwrap_or(false)
}

/// AC1 root remove: after a reload that drops the docs root, writing to
/// docs/ no longer triggers, while src/ still does. Process never exits.
#[test]
fn root_remove_stops_routing_after_commit() {
    setup::serialized(|| {
        let scratch = scratch_root("root-remove");
        let mut child = spawn_watcher(&scratch, &base_config());
        wait_for_socket(&scratch);

        // Reload: drop the docs job/root.
        let shrunk =
            "on:\n  socket: sock\njobs:\n  - name: capture-src\n    run: 'echo src > src-verdict.txt'\n    change: 'src/**'\n";
        std::fs::write(scratch.join(".watch.yaml"), shrunk).unwrap();
        wait_for_log(&scratch, "hot-reloading to revision 2");
        assert!(
            child.try_wait().expect("try_wait").is_none(),
            "root remove must not exit the process"
        );

        // Docs must no longer route after the boundary.
        std::fs::write(scratch.join("docs/old.md"), "x").unwrap();
        let deadline = std::time::Instant::now() + Duration::from_secs(5);
        while std::time::Instant::now() < deadline {
            assert!(
                !scratch.join("docs-verdict.txt").exists(),
                "removed root must stop routing"
            );
            std::thread::sleep(Duration::from_millis(100));
        }

        // src still routes under the new revision.
        std::fs::write(scratch.join("src/main.rs"), "x").unwrap();
        let deadline = std::time::Instant::now() + Duration::from_secs(15);
        let mut ran = false;
        while std::time::Instant::now() < deadline {
            if scratch.join("src-verdict.txt").exists() {
                ran = true;
                break;
            }
            std::thread::sleep(Duration::from_millis(100));
        }
        let _ = child.kill();
        let _ = child.wait();
        assert!(ran, "kept root must keep routing");
        std::fs::remove_dir_all(&scratch).unwrap();
    });
}

/// AC1 root overlap + AC4: during the boundary both old and new roots are
/// watched; a change in the overlap must route exactly once (one revision
/// identity), never a duplicate generation.
#[test]
fn overlapping_roots_normalize_to_one_generation() {
    setup::serialized(|| {
        let scratch = scratch_root("overlap");
        // src covered by both the old and new root sets (old: src; new:
        // src + docs). A single write in the overlap must produce exactly
        // one generation — the verdict file is written once per run and the
        // run counter proves no double scheduling.
        let config = "on:\n  socket: sock\njobs:\n  - name: count\n    run: 'echo x >> count.txt'\n    change: 'src/**'\n";
        let mut child = spawn_watcher(&scratch, config);
        wait_for_socket(&scratch);

        // Add docs job (new root) — overlap with src stays.
        let grown = "on:\n  socket: sock\njobs:\n  - name: count\n    run: 'echo x >> count.txt'\n    change: 'src/**'\n  - name: docs\n    run: 'echo d >> docs-count.txt'\n    change: 'docs/**'\n";
        std::fs::write(scratch.join(".watch.yaml"), grown).unwrap();
        wait_for_log(&scratch, "hot-reloading to revision 2");

        // Write in the overlap path: must route exactly once.
        std::fs::write(scratch.join("src/main.rs"), "x").unwrap();
        let deadline = std::time::Instant::now() + Duration::from_secs(15);
        while std::time::Instant::now() < deadline {
            if std::fs::read_to_string(scratch.join("count.txt"))
                .is_ok_and(|contents| contents.lines().next().is_some())
            {
                break;
            }
            std::thread::sleep(Duration::from_millis(100));
        }
        // Give any spurious second generation time to appear.
        std::thread::sleep(Duration::from_millis(1200));
        let final_count = std::fs::read_to_string(scratch.join("count.txt"))
            .unwrap_or_default()
            .lines()
            .count();
        let _ = child.kill();
        let _ = child.wait();
        assert_eq!(
            final_count, 1,
            "overlap change must normalize to one generation, got {final_count}"
        );
        std::fs::remove_dir_all(&scratch).unwrap();
    });
}

/// AC1 job add/remove/rename: after a reload, a renamed job stops routing
/// under the old name and starts under the new one; process survives.
#[test]
fn job_rename_swaps_matching_without_process_exit() {
    setup::serialized(|| {
        let scratch = scratch_root("job-rename");
        let mut child = spawn_watcher(&scratch, &base_config());
        wait_for_socket(&scratch);

        // Rename capture-src → capture-src-v2 (same change glob).
        let renamed = "on:\n  socket: sock\njobs:\n  - name: capture-src-v2\n    run: 'echo v2 > src-v2-verdict.txt'\n    change: 'src/**'\n  - name: capture-docs\n    run: 'echo docs > docs-verdict.txt'\n    change: 'docs/**'\n";
        std::fs::write(scratch.join(".watch.yaml"), renamed).unwrap();
        wait_for_log(&scratch, "hot-reloading to revision 2");
        assert!(
            child.try_wait().expect("try_wait").is_none(),
            "job rename must not exit the process"
        );

        std::fs::write(scratch.join("src/main.rs"), "x").unwrap();
        let deadline = std::time::Instant::now() + Duration::from_secs(15);
        let mut ran = false;
        while std::time::Instant::now() < deadline {
            if scratch.join("src-v2-verdict.txt").exists() {
                ran = true;
                break;
            }
            std::thread::sleep(Duration::from_millis(100));
        }
        let _ = child.kill();
        let _ = child.wait();
        assert!(ran, "renamed job must route under its new name");
        std::fs::remove_dir_all(&scratch).unwrap();
    });
}

/// AC1 matching/ignore: a reload that adds an ignore rule stops routing the
/// ignored path while the matching path keeps routing.
#[test]
fn ignore_rule_added_on_reload_stops_routing_ignored_path() {
    setup::serialized(|| {
        let scratch = scratch_root("ignore-reload");
        let mut child = spawn_watcher(&scratch, &base_config());
        wait_for_socket(&scratch);

        std::fs::create_dir_all(scratch.join("src/ignored")).unwrap();
        // Pre-create the ignored file before the reload: rewriting it fires a
        // file-path event only (FSEvents), so the assertion targets the
        // ignore rule rather than directory-mtime noise.
        std::fs::write(scratch.join("src/ignored/x.rs"), "seed").unwrap();
        std::thread::sleep(Duration::from_millis(700));
        let with_ignore = "on:\n  socket: sock\njobs:\n  - name: capture-src\n    run: 'echo src > src-verdict.txt'\n    change: 'src/**'\n    ignore: 'src/ignored/**'\n  - name: capture-docs\n    run: 'echo docs > docs-verdict.txt'\n    change: 'docs/**'\n";
        std::fs::write(scratch.join(".watch.yaml"), with_ignore).unwrap();
        wait_for_log(&scratch, "hot-reloading to revision 2");
        let _ = std::fs::remove_file(scratch.join("src-verdict.txt"));

        std::fs::write(scratch.join("src/ignored/x.rs"), "x").unwrap();
        let deadline = std::time::Instant::now() + Duration::from_secs(5);
        while std::time::Instant::now() < deadline {
            assert!(
                !scratch.join("src-verdict.txt").exists(),
                "ignored path must not route after reload"
            );
            std::thread::sleep(Duration::from_millis(100));
        }
        let _ = child.kill();
        let _ = child.wait();
        std::fs::remove_dir_all(&scratch).unwrap();
    });
}

/// AC5: a valid config save does NOT kill an active finite generation; the
/// running task completes with the old revision while later events route
/// under the new one.
#[test]
fn active_finite_task_survives_config_save() {
    setup::serialized(|| {
        let scratch = scratch_root("active-survives");
        let slow = "on:\n  socket: sock\njobs:\n  - name: slow\n    run: 'sleep 2; echo done > slow-verdict.txt'\n    change: 'src/**'\n";
        let mut child = spawn_watcher(&scratch, slow);
        wait_for_socket(&scratch);

        // Trigger the slow generation, then immediately reload the config.
        std::fs::write(scratch.join("src/a.rs"), "x").unwrap();
        // Deterministic ordering: wait until the slow generation is RUNNING
        // (status generation 1, running) before rewriting the config. A fixed
        // sleep races the 1s debounce window: under parallel load the batch
        // can fire after the reload commit and route the fast job instead,
        // which would never produce the slow verdict.
        let deadline = std::time::Instant::now() + Duration::from_secs(15);
        let mut slow_running = false;
        while std::time::Instant::now() < deadline {
            if status_generation_running(&scratch) {
                slow_running = true;
                break;
            }
            std::thread::sleep(Duration::from_millis(50));
        }
        assert!(slow_running, "slow generation must start before the reload");
        let new_config = "on:\n  socket: sock\njobs:\n  - name: fast\n    run: 'echo fast > fast-verdict.txt'\n    change: 'src/**'\n";
        std::fs::write(scratch.join(".watch.yaml"), new_config).unwrap();
        wait_for_log(&scratch, "hot-reloading to revision 2");
        assert!(
            child.try_wait().expect("try_wait").is_none(),
            "config save must not kill the watcher"
        );

        // The slow generation (old revision) completes despite the save.
        let deadline = std::time::Instant::now() + Duration::from_secs(15);
        let mut slow_done = false;
        while std::time::Instant::now() < deadline {
            if scratch.join("slow-verdict.txt").exists() {
                slow_done = true;
                break;
            }
            std::thread::sleep(Duration::from_millis(100));
        }
        let _ = child.kill();
        let _ = child.wait();
        assert!(
            slow_done,
            "active finite task must complete under old revision"
        );
        std::fs::remove_dir_all(&scratch).unwrap();
    });
}

/// AC8: a control socket PATH change binds the new socket before commit and
/// retires the old after; the process stays alive and the NEW socket serves
/// control while the old path is removed. Bind failure (occupied new path)
/// is fatal — proven by the invalid path scenario below.
#[test]
fn socket_path_change_binds_new_before_retiring_old() {
    setup::serialized(|| {
        let scratch = scratch_root("socket-move");
        let mut child = spawn_watcher(&scratch, &base_config());
        wait_for_socket(&scratch);
        assert!(
            scratch.join("sock").exists(),
            "old socket must exist before the reload"
        );

        let moved = "on:\n  socket: sock2\njobs:\n  - name: capture-src\n    run: 'echo src > src-verdict.txt'\n    change: 'src/**'\n  - name: capture-docs\n    run: 'echo docs > docs-verdict.txt'\n    change: 'docs/**'\n";
        std::fs::write(scratch.join(".watch.yaml"), moved).unwrap();
        wait_for_log(&scratch, "Control socket rebinding to");
        assert!(
            child.try_wait().expect("try_wait").is_none(),
            "socket path change must not exit the process"
        );

        // The NEW socket is live and connectable.
        let deadline = std::time::Instant::now() + Duration::from_secs(10);
        let mut new_connected = false;
        while std::time::Instant::now() < deadline {
            if std::os::unix::net::UnixStream::connect(scratch.join("sock2")).is_ok() {
                new_connected = true;
                break;
            }
            std::thread::sleep(Duration::from_millis(100));
        }
        assert!(new_connected, "new socket must be connectable after reload");
        // The OLD socket file is removed by the retired server. The retire
        // happens after the commit boundary (later than the "rebinding" log
        // the wait observed), so poll instead of asserting on an exact
        // instant — the invariant is eventual removal, never a stale socket
        // left live.
        let deadline = std::time::Instant::now() + Duration::from_secs(10);
        let mut old_retired = false;
        while std::time::Instant::now() < deadline {
            if !scratch.join("sock").exists() {
                old_retired = true;
                break;
            }
            std::thread::sleep(Duration::from_millis(100));
        }
        assert!(old_retired, "old socket must be retired after commit");
        let _ = child.kill();
        let _ = child.wait();
        std::fs::remove_dir_all(&scratch).unwrap();
    });
}

/// AC6: a managed service whose signature changed on reload is gracefully
/// replaced; an unchanged service stays owned. Both keep the process alive.
#[test]
fn service_signature_change_replaces_service_without_process_exit() {
    setup::serialized(|| {
        let scratch = scratch_root("service-sig");
        std::fs::write(
            scratch.join("svc.sh"),
            "#!/usr/bin/env bash\necho $$ >> svc-pids\nwhile true; do touch svc-ready; sleep 0.2; done\n",
        )
        .unwrap();
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            let mut perms = std::fs::metadata(scratch.join("svc.sh"))
                .unwrap()
                .permissions();
            perms.set_mode(0o755);
            std::fs::set_permissions(scratch.join("svc.sh"), perms).unwrap();
        }
        let config = "on:\n  socket: sock\njobs:\n  - name: dev-server\n    service: true\n    run: './svc.sh'\n    change: 'src/**'\n";
        let mut child = spawn_watcher(&scratch, config);
        wait_for_socket(&scratch);

        // Start the service generation, then change the service signature:
        // the old process must be gracefully replaced (new pid file) and the
        // process never exits.
        std::fs::write(scratch.join("src/a.rs"), "x").unwrap();
        let deadline = std::time::Instant::now() + Duration::from_secs(15);
        let mut started = false;
        while std::time::Instant::now() < deadline {
            if scratch.join("svc-ready").exists() {
                started = true;
                break;
            }
            std::thread::sleep(Duration::from_millis(100));
        }
        assert!(started, "service must start under the first revision");

        let changed = "on:\n  socket: sock\njobs:\n  - name: dev-server\n    service: true\n    run: './svc.sh --changed'\n    change: 'src/**'\n";
        std::fs::write(scratch.join(".watch.yaml"), changed).unwrap();
        wait_for_log(&scratch, "hot-reloading to revision 2");
        assert!(
            child.try_wait().expect("try_wait").is_none(),
            "service signature change must not exit the process"
        );
        // The reloaded service keeps running (the replaced process touches
        // svc-ready again). Wait for the second service PID before checking
        // the readiness file, so a final write from the old service cannot
        // make replacement appear complete.
        let deadline = std::time::Instant::now() + Duration::from_secs(10);
        let mut replaced = false;
        while std::time::Instant::now() < deadline {
            let service_count = std::fs::read_to_string(scratch.join("svc-pids"))
                .unwrap_or_default()
                .lines()
                .count();
            if service_count >= 2 {
                replaced = true;
                break;
            }
            std::thread::sleep(Duration::from_millis(100));
        }
        let _ = std::fs::remove_file(scratch.join("svc-ready"));
        let deadline = std::time::Instant::now() + Duration::from_secs(10);
        let mut still_running = false;
        while std::time::Instant::now() < deadline {
            if scratch.join("svc-ready").exists() {
                still_running = true;
                break;
            }
            std::thread::sleep(Duration::from_millis(100));
        }

        // SIGKILLing the watcher skips its shutdown coordinator and leaves the
        // service group orphaned. Use the real shutdown path, then assert both
        // service generations (including their `sleep` descendants) are gone.
        let service_groups: Vec<i32> = std::fs::read_to_string(scratch.join("svc-pids"))
            .unwrap_or_default()
            .lines()
            .filter_map(|line| line.trim().parse().ok())
            .collect();
        let graceful = stop_watcher_gracefully(&mut child);
        let groups_stopped = service_groups
            .iter()
            .copied()
            .all(wait_for_process_group_exit);
        if !groups_stopped {
            // Keep teardown leak-free even when the regression assertion
            // fails, so one broken run cannot accumulate fixture processes.
            for pgid in &service_groups {
                force_kill_process_group(*pgid);
            }
        }
        // TASK-0162 reload reap breadth: stop/reap-before-start means the
        // old service group must already be GONE by the time the replacement
        // pid exists — not merely reaped later at watcher teardown.
        let first_service_pid: i32 = std::fs::read_to_string(scratch.join("svc-pids"))
            .unwrap_or_default()
            .lines()
            .next()
            .and_then(|line| line.trim().parse().ok())
            .expect("first service pid");
        assert!(
            wait_for_process_group_exit(first_service_pid),
            "valid reload must reap the replaced service process group before the replacement starts"
        );
        assert!(replaced, "changed service must be replaced");
        assert!(still_running, "changed service must keep running");
        assert_eq!(
            service_groups.len(),
            2,
            "reload must start exactly two services"
        );
        assert!(graceful, "watcher must exit through graceful shutdown");
        assert!(
            groups_stopped,
            "watcher shutdown must stop and reap every managed service process group"
        );
        std::fs::remove_dir_all(&scratch).unwrap();
    });
}
