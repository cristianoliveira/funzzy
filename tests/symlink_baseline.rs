//! TASK-0166: spawned-watcher proof for symlink-safe baseline seeding.
//!
//! The broad `**/*.txt` pattern selects the workspace as a baseline root. A
//! directory symlink points back to that root, so readiness proves the initial
//! baseline does not follow symlink cycles.

#![cfg(all(feature = "test-integration", unix))]

use std::os::unix::net::UnixStream;
use std::path::PathBuf;
use std::sync::atomic::{AtomicU32, Ordering};

#[path = "./common/lib.rs"]
mod setup;

static SOCKET_COUNTER: AtomicU32 = AtomicU32::new(0);

fn socket_path() -> PathBuf {
    let counter = SOCKET_COUNTER.fetch_add(1, Ordering::SeqCst);
    std::env::temp_dir().join(format!(
        "fzzx-baseline-sock-{}-{counter}",
        std::process::id()
    ))
}

#[test]
fn broad_root_with_ancestor_symlink_reaches_watcher_readiness() {
    setup::with_output("symlink-baseline-cycle.log", |fzz_cmd, _, fixture| {
        let socket = socket_path();
        std::fs::create_dir_all(fixture.join("cycle/nested")).unwrap();
        std::fs::write(fixture.join("cycle/nested/existing.txt"), "existing").unwrap();
        std::os::unix::fs::symlink(fixture, fixture.join("cycle/back-to-root")).unwrap();
        let config = format!(
            "on:\n  socket: '{}'\njobs:\n  - name: ready\n    run: 'echo ready > ready.marker'\n    change: '**/*.txt'\n    run_on_init: true\n",
            socket.display()
        );
        std::fs::write(fixture.join(".watch.yaml"), config).unwrap();

        let mut child = fzz_cmd
            .args(["watch", "-v"])
            .spawn()
            .expect("watcher should start");
        defer!({
            let pid = child.id().to_string();
            let _ = std::process::Command::new("kill")
                .args(["-INT", &pid])
                .status();
            for _ in 0..150 {
                if child.try_wait().ok().flatten().is_some() {
                    break;
                }
                std::thread::sleep(std::time::Duration::from_millis(20));
            }
            if child.try_wait().ok().flatten().is_none() {
                let _ = child.kill();
                let _ = child.wait();
            }
            let _ = std::fs::remove_file(&socket);
        });

        wait_until!(
            { UnixStream::connect(&socket).is_ok() },
            "watcher readiness control socket"
        );
        wait_until!(
            { fixture.join("ready.marker").exists() },
            "initial finite job after baseline"
        );
        assert!(
            UnixStream::connect(&socket).is_ok(),
            "control socket must remain available after initialization"
        );
        assert!(fixture.join("cycle/nested/existing.txt").exists());
    });
}
