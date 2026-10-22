//! Watch-mode proof for execution defaults on service starts, readiness, and close hooks.

#![cfg(all(feature = "test-integration", unix))]

use nix::sys::signal::{kill, Signal};
use nix::unistd::Pid;
use std::path::PathBuf;
use std::process::{Child, Command, Stdio};
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};

struct Watcher(Child);

impl Drop for Watcher {
    fn drop(&mut self) {
        let _ = kill(Pid::from_raw(self.0.id() as i32), Signal::SIGTERM);
        for _ in 0..50 {
            if self.0.try_wait().ok().flatten().is_some() {
                return;
            }
            std::thread::sleep(Duration::from_millis(100));
        }
        let _ = self.0.kill();
        let _ = self.0.wait();
    }
}

fn wait_for(path: &PathBuf) {
    let deadline = Instant::now() + Duration::from_secs(10);
    while !path.exists() {
        assert!(
            Instant::now() < deadline,
            "timed out waiting for {}",
            path.display()
        );
        std::thread::sleep(Duration::from_millis(10));
    }
}

#[test]
fn execution_environment_reaches_service_readiness_and_close_in_both_watch_modes() {
    for non_blocking in [false, true] {
        let unique = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap()
            .as_nanos();
        let root = std::env::temp_dir().join(format!(
            "funzzy-execution-env-watch-{}-{unique}-{non_blocking}",
            std::process::id()
        ));
        std::fs::create_dir_all(&root).unwrap();
        let socket = if non_blocking {
            "  socket: watcher.sock\n"
        } else {
            ""
        };
        std::fs::write(
            root.join(".watch.yaml"),
            format!(
                "on:\n  change: '**/*'\n{socket}execution:\n  env:\n    WATCH_VALUE: inherited-from-execution\nhooks:\n  close: 'printf \"%s\" \"$WATCH_VALUE\" > closed.txt'\njobs:\n  - name: server\n    service: true\n    env:\n      WATCH_VALUE: from-job\n    run: 'test \"$WATCH_VALUE\" = from-job || exit 9; while true; do sleep 1; done'\n    run_on_init: true\n    readiness:\n      run: 'test \"$WATCH_VALUE\" = from-job && printf ready > readiness.txt'\n      timeout: 5s\n      interval: 50ms\n"
            ),
        )
        .unwrap();

        let child = Command::new(env!("CARGO_BIN_EXE_fzz"))
            .current_dir(&root)
            .env("FUNZZY_COLORED", "false")
            .stdin(Stdio::null())
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .spawn()
            .unwrap();
        let mut watcher = Watcher(child);
        wait_for(&root.join("readiness.txt"));
        assert_eq!(
            std::fs::read_to_string(root.join("readiness.txt")).unwrap(),
            "ready"
        );

        kill(Pid::from_raw(watcher.0.id() as i32), Signal::SIGTERM).unwrap();
        let _status = watcher.0.wait().unwrap();
        wait_for(&root.join("closed.txt"));
        assert_eq!(
            std::fs::read_to_string(root.join("closed.txt")).unwrap(),
            "inherited-from-execution"
        );
        std::fs::remove_dir_all(root).unwrap();
    }
}
