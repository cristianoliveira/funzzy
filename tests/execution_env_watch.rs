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
fn blocking_watch_uses_reloaded_execution_environment_for_the_next_job() {
    let unique = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap()
        .as_nanos();
    let root = std::env::temp_dir().join(format!(
        "funzzy-execution-env-reload-{}-{unique}",
        std::process::id()
    ));
    std::fs::create_dir_all(root.join("triggers")).unwrap();
    std::fs::write(
        root.join(".watch.yaml"),
        "on:\n  change: 'triggers/**'\nexecution:\n  env:\n    PROJECT_VALUE: first\njobs:\n  - name: echo\n    run: 'printf \"%s\\n\" \"$PROJECT_VALUE\" >> values.txt'\n    run_on_init: true\n",
    )
    .unwrap();
    let log_path = root.join("watcher.log");
    let log = std::fs::File::create(&log_path).unwrap();
    let child = Command::new(env!("CARGO_BIN_EXE_fzz"))
        .current_dir(&root)
        .env("FUNZZY_COLORED", "false")
        .stdin(Stdio::null())
        .stdout(Stdio::from(log))
        .stderr(Stdio::null())
        .spawn()
        .unwrap();
    let mut watcher = Watcher(child);
    let values_path = root.join("values.txt");
    wait_for(&values_path);
    assert_eq!(std::fs::read_to_string(&values_path).unwrap(), "first\n");

    std::fs::write(
        root.join(".watch.yaml"),
        "on:\n  change: 'triggers/**'\nexecution:\n  env:\n    PROJECT_VALUE: second\njobs:\n  - name: echo\n    run: 'printf \"%s\\n\" \"$PROJECT_VALUE\" >> values.txt'\n    run_on_init: true\n",
    )
    .unwrap();
    let deadline = Instant::now() + Duration::from_secs(10);
    while !std::fs::read_to_string(&log_path)
        .unwrap_or_default()
        .contains("Config change is valid; hot-reloading to revision 2.")
    {
        assert!(
            Instant::now() < deadline,
            "execution.env reload did not commit"
        );
        std::thread::sleep(Duration::from_millis(10));
    }

    std::fs::write(root.join("triggers/second"), "trigger").unwrap();
    let deadline = Instant::now() + Duration::from_secs(10);
    loop {
        let values = std::fs::read_to_string(&values_path).unwrap_or_default();
        if values.lines().count() >= 2 {
            assert_eq!(values, "first\nsecond\n");
            break;
        }
        assert!(
            Instant::now() < deadline,
            "second job did not finish; values={values:?}"
        );
        std::thread::sleep(Duration::from_millis(10));
    }

    kill(Pid::from_raw(watcher.0.id() as i32), Signal::SIGTERM).unwrap();
    let _ = watcher.0.wait();
    std::fs::remove_dir_all(root).unwrap();
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
                "on:\n  change: '**/*'\n{socket}execution:\n  env:\n    WATCH_VALUE: inherited-from-execution\nhooks:\n  close: 'printf \"%s\" \"$WATCH_VALUE\" > closed.txt'\njobs:\n  - name: server\n    service: true\n    run: 'test \"$WATCH_VALUE\" = inherited-from-execution || exit 9; while true; do sleep 1; done'\n    run_on_init: true\n    readiness:\n      run: 'test \"$WATCH_VALUE\" = inherited-from-execution && printf ready > readiness.txt'\n      timeout: 5s\n      interval: 50ms\n"
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
