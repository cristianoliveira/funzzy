//! TASK-0102: installed-binary proof for watcher `on.close` lifecycle.
//! Real signals/process groups/config reloads only — no mocked signal claims.

#[path = "./common/lib.rs"]
mod setup;

#[cfg(feature = "test-integration")]
mod lifecycle {
    use super::setup;
    use serde_json::Value;
    use std::fs;
    use std::path::{Path, PathBuf};
    use std::process::{Child, Command, ExitStatus, Stdio};
    use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};

    fn scratch(label: &str) -> PathBuf {
        let unique = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .expect("clock after epoch")
            .as_nanos();
        let root = std::env::temp_dir().join(format!(
            "funzzy-close-hook-{}-{}-{}",
            std::process::id(),
            unique,
            label
        ));
        let _ = fs::remove_dir_all(&root);
        fs::create_dir_all(&root).expect("create scratch");
        root
    }

    fn spawn(root: &Path, config: &str, env: &[(&str, &str)]) -> Child {
        fs::write(root.join(".watch.yaml"), config).expect("write config");
        let stdout = fs::File::create(root.join("child.out")).expect("stdout file");
        let stderr = fs::File::create(root.join("child.err")).expect("stderr file");
        let mut command = Command::new(env!("CARGO_BIN_EXE_fzz"));
        command
            .current_dir(root)
            .env("FUNZZY_COLORED", "false")
            .env("_TEST_FUNZZY_COLORED", "false")
            .stdout(Stdio::from(stdout))
            .stderr(Stdio::from(stderr));
        for (key, value) in env {
            command.env(key, value);
        }
        command.spawn().expect("spawn fzz")
    }

    fn wait_until(deadline: Duration, description: &str, mut ready: impl FnMut() -> bool) {
        let end = Instant::now() + deadline;
        while Instant::now() < end {
            if ready() {
                return;
            }
            std::thread::sleep(Duration::from_millis(20));
        }
        panic!("timed out waiting for {description}");
    }

    fn wait_ready(root: &Path) {
        wait_until(Duration::from_secs(20), "watcher readiness", || {
            fs::read_to_string(root.join("child.out"))
                .map(|out| out.contains("Watching...") || out.contains("Running on init"))
                .unwrap_or(false)
        });
    }

    fn signal(child: &Child, name: &str) {
        let status = Command::new("kill")
            .arg(format!("-{name}"))
            .arg(child.id().to_string())
            .status()
            .expect("send signal");
        assert!(status.success(), "failed to send {name}");
    }

    fn wait_exit(child: &mut Child, bound: Duration) -> ExitStatus {
        let end = Instant::now() + bound;
        loop {
            if let Some(status) = child.try_wait().expect("poll child") {
                return status;
            }
            assert!(
                Instant::now() < end,
                "watcher did not exit within {bound:?}"
            );
            std::thread::sleep(Duration::from_millis(20));
        }
    }

    fn alive(pid: u32) -> bool {
        Command::new("kill")
            .arg("-0")
            .arg(pid.to_string())
            .output()
            .map(|out| out.status.success())
            .unwrap_or(false)
    }

    fn read_pid(path: &Path) -> u32 {
        fs::read_to_string(path)
            .expect("pid file")
            .trim()
            .parse()
            .expect("numeric pid")
    }

    struct Cleanup {
        root: PathBuf,
        child: Option<Child>,
    }

    impl Cleanup {
        fn new(root: PathBuf, child: Child) -> Self {
            Self {
                root,
                child: Some(child),
            }
        }

        fn child(&self) -> &Child {
            self.child.as_ref().unwrap()
        }

        fn child_mut(&mut self) -> &mut Child {
            self.child.as_mut().unwrap()
        }

        fn finish(mut self) {
            if let Some(mut child) = self.child.take() {
                let _ = child.kill();
                let _ = child.wait();
            }
            let _ = fs::remove_dir_all(&self.root);
        }
    }

    impl Drop for Cleanup {
        fn drop(&mut self) {
            if let Some(child) = self.child.as_mut() {
                let _ = child.kill();
                let _ = child.wait();
            }
            let _ = fs::remove_dir_all(&self.root);
        }
    }

    const IDLE: &str = "jobs:\n  - name: idle\n    run: echo idle\n    change: '**/*.rs'\n";

    #[test]
    fn sigint_and_sigterm_run_close_exactly_once_and_preserve_exit_codes() {
        setup::serialized(|| {
            for (name, expected) in [("INT", 130), ("TERM", 143)] {
                let root = scratch(&format!("signal-{name}"));
                let config = format!("hooks:\n  close: \"echo close >> close.log\"\n{IDLE}");
                let child = spawn(&root, &config, &[]);
                let mut cleanup = Cleanup::new(root.clone(), child);
                wait_ready(&root);
                signal(cleanup.child(), name);
                let status = wait_exit(cleanup.child_mut(), Duration::from_secs(10));
                assert_eq!(status.code(), Some(expected));
                assert_eq!(
                    fs::read_to_string(root.join("close.log")).unwrap(),
                    "close\n",
                    "{name} must run one close hook"
                );
                cleanup.finish();
            }
        });
    }

    #[test]
    fn close_runs_after_active_finite_job_and_service_are_reaped() {
        setup::serialized(|| {
            let root = scratch("ordering");
            let config = r#"execution:
  concurrency: 2
hooks:
  close: "if kill -0 $(cat finite.pid) 2>/dev/null || kill -0 $(cat service.pid) 2>/dev/null; then echo alive; else echo reaped; fi > order.txt"
jobs:
  - name: finite
    run: "echo $$ > finite.pid; trap 'exit 0' TERM INT; while true; do sleep 1; done"
    run_on_init: true
    parallel: active
  - name: service
    run: "echo $$ > service.pid; trap 'exit 0' TERM INT; while true; do sleep 1; done"
    run_on_init: true
    service: true
    parallel: active
"#;
            let child = spawn(&root, config, &[]);
            let mut cleanup = Cleanup::new(root.clone(), child);
            wait_until(
                Duration::from_secs(20),
                "both owned process pid files",
                || root.join("finite.pid").exists() && root.join("service.pid").exists(),
            );
            let finite = read_pid(&root.join("finite.pid"));
            let service = read_pid(&root.join("service.pid"));
            assert!(alive(finite) && alive(service));

            signal(cleanup.child(), "TERM");
            assert_eq!(
                wait_exit(cleanup.child_mut(), Duration::from_secs(12)).code(),
                Some(143)
            );
            assert_eq!(
                fs::read_to_string(root.join("order.txt")).unwrap().trim(),
                "reaped"
            );
            assert!(!alive(finite), "finite job orphaned");
            assert!(!alive(service), "managed service orphaned");
            cleanup.finish();
        });
    }

    #[test]
    fn hook_failure_and_spawn_failure_are_visible_without_replacing_signal_exit() {
        setup::serialized(|| {
            for (label, close, env, expected_stderr) in [
                (
                    "failure",
                    "echo hook-command-failed >&2; exit 7",
                    vec![],
                    "close hook failed",
                ),
                (
                    "spawn",
                    "echo unreachable",
                    vec![("SHELL", "/definitely/missing-shell")],
                    "close hook failed",
                ),
            ] {
                let root = scratch(label);
                let config = format!("hooks:\n  close: \"{close}\"\n{IDLE}");
                let child = spawn(&root, &config, &env);
                let mut cleanup = Cleanup::new(root.clone(), child);
                wait_ready(&root);
                signal(cleanup.child(), "TERM");
                assert_eq!(
                    wait_exit(cleanup.child_mut(), Duration::from_secs(10)).code(),
                    Some(143)
                );
                let stderr = fs::read_to_string(root.join("child.err")).unwrap_or_default();
                assert!(stderr.contains(expected_stderr), "{label}: {stderr}");
                cleanup.finish();
            }
        });
    }

    #[test]
    fn hook_timeout_reaps_descendant_and_keeps_original_exit() {
        setup::serialized(|| {
            let root = scratch("timeout-child");
            let config =
                format!("hooks:\n  close: \"sleep 30 & echo $! > hook-child.pid; wait\"\n{IDLE}");
            let child = spawn(&root, &config, &[("FUNZZY_CANCEL_GRACE_MS", "500")]);
            let mut cleanup = Cleanup::new(root.clone(), child);
            wait_ready(&root);
            let started = Instant::now();
            signal(cleanup.child(), "TERM");
            assert_eq!(
                wait_exit(cleanup.child_mut(), Duration::from_secs(5)).code(),
                Some(143)
            );
            assert!(started.elapsed() < Duration::from_secs(5));
            let descendant = read_pid(&root.join("hook-child.pid"));
            assert!(!alive(descendant), "timed-out hook orphaned {descendant}");
            let stderr = fs::read_to_string(root.join("child.err")).unwrap_or_default();
            assert!(stderr.contains("close hook timed out"), "{stderr}");
            cleanup.finish();
        });
    }

    #[test]
    fn second_signal_cancels_hook_without_duplicate_execution_or_reason_change() {
        setup::serialized(|| {
            let root = scratch("second-signal");
            let config = format!(
                "hooks:\n  close: \"echo close >> close.log; trap '' TERM INT; sleep 30\"\n{IDLE}"
            );
            let child = spawn(&root, &config, &[("FUNZZY_CANCEL_GRACE_MS", "1000")]);
            let mut cleanup = Cleanup::new(root.clone(), child);
            wait_ready(&root);
            signal(cleanup.child(), "TERM");
            wait_until(Duration::from_secs(5), "close hook start", || {
                root.join("close.log").exists()
            });
            signal(cleanup.child(), "INT");
            assert_eq!(
                wait_exit(cleanup.child_mut(), Duration::from_secs(5)).code(),
                Some(143),
                "first signal reason must win"
            );
            assert_eq!(
                fs::read_to_string(root.join("close.log")).unwrap(),
                "close\n"
            );
            cleanup.finish();
        });
    }

    #[test]
    fn valid_reload_uses_latest_hook_and_invalid_reload_uses_last_valid_hook() {
        setup::serialized(|| {
            // Valid commit replaces future hook.
            let root = scratch("reload-valid");
            let initial = format!("hooks:\n  close: \"echo old > old.txt\"\n{IDLE}");
            let child = spawn(&root, &initial, &[]);
            let mut cleanup = Cleanup::new(root.clone(), child);
            wait_ready(&root);
            fs::write(
                root.join(".watch.yaml"),
                format!("hooks:\n  close: \"echo new > new.txt\"\n{IDLE}"),
            )
            .unwrap();
            wait_until(Duration::from_secs(20), "valid reload commit", || {
                fs::read_to_string(root.join("child.out"))
                    .map(|out| out.contains("hot-reloading to revision 2"))
                    .unwrap_or(false)
            });
            signal(cleanup.child(), "TERM");
            assert_eq!(
                wait_exit(cleanup.child_mut(), Duration::from_secs(10)).code(),
                Some(143)
            );
            assert!(root.join("new.txt").exists());
            assert!(!root.join("old.txt").exists());
            cleanup.finish();

            // Invalid candidate never replaces last committed hook; fatal
            // config exit stays nonzero and hook runs once.
            let root = scratch("reload-invalid");
            let stable = format!("hooks:\n  close: \"echo stable >> stable.log\"\n{IDLE}");
            let child = spawn(&root, &stable, &[]);
            let mut cleanup = Cleanup::new(root.clone(), child);
            wait_ready(&root);
            fs::write(root.join(".watch.yaml"), "jobs: [unclosed").unwrap();
            let status = wait_exit(cleanup.child_mut(), Duration::from_secs(20));
            assert_eq!(status.code(), Some(1));
            assert_eq!(
                fs::read_to_string(root.join("stable.log")).unwrap(),
                "stable\n"
            );
            cleanup.finish();
        });
    }

    #[test]
    fn absent_hook_keeps_shutdown_fast_and_finite_commands_never_run_close() {
        setup::serialized(|| {
            let root = scratch("absent");
            let child = spawn(&root, IDLE, &[]);
            let mut cleanup = Cleanup::new(root.clone(), child);
            wait_ready(&root);
            let started = Instant::now();
            signal(cleanup.child(), "TERM");
            assert_eq!(
                wait_exit(cleanup.child_mut(), Duration::from_secs(5)).code(),
                Some(143)
            );
            assert!(started.elapsed() < Duration::from_secs(5));
            cleanup.finish();

            let root = scratch("finite");
            let config = "hooks:\n  close: \"echo forbidden > close.txt\"\njobs:\n  - name: once\n    run: echo once\n    change: '**/*'\n";
            fs::write(root.join(".watch.yaml"), config).unwrap();
            for args in [vec!["run", "once"], vec!["check"]] {
                let status = Command::new(env!("CARGO_BIN_EXE_fzz"))
                    .current_dir(&root)
                    .env("FUNZZY_COLORED", "false")
                    .args(args)
                    .status()
                    .unwrap();
                assert!(status.success());
                assert!(!root.join("close.txt").exists());
            }
            fs::remove_dir_all(root).unwrap();
        });
    }

    #[test]
    fn close_written_file_cannot_schedule_another_generation() {
        setup::serialized(|| {
            let root = scratch("no-feedback");
            let config = "hooks:\n  close: \"echo close > close-trigger.txt\"\njobs:\n  - name: feedback\n    run: \"echo generation >> generations.log\"\n    change: '**/*.txt'\n";
            let child = spawn(&root, config, &[]);
            let mut cleanup = Cleanup::new(root.clone(), child);
            wait_ready(&root);
            signal(cleanup.child(), "TERM");
            assert_eq!(
                wait_exit(cleanup.child_mut(), Duration::from_secs(10)).code(),
                Some(143)
            );
            assert!(root.join("close-trigger.txt").exists());
            assert!(!root.join("generations.log").exists());
            cleanup.finish();
        });
    }

    #[test]
    fn schema_init_and_check_expose_and_validate_close() {
        setup::serialized(|| {
            let root = scratch("discovery");
            let schema = Command::new(env!("CARGO_BIN_EXE_fzz"))
                .current_dir(&root)
                .args(["config", "schema", "--format", "json"])
                .output()
                .unwrap();
            assert!(schema.status.success());
            let json: Value = serde_json::from_slice(&schema.stdout).unwrap();
            assert_eq!(
                json["$defs"]["hooks"]["properties"]["close"]["type"],
                "string"
            );

            let init = Command::new(env!("CARGO_BIN_EXE_fzz"))
                .current_dir(&root)
                .arg("init")
                .status()
                .unwrap();
            assert!(init.success());
            let generated = fs::read_to_string(root.join(".watch.yaml")).unwrap();
            assert!(generated.contains("# close:"));

            for invalid in [
                "hooks:\n  close: [a, b]\njobs:\n  - name: a\n    run: echo a\n    change: '**/*'\n",
                "hooks:\n  close: ''\njobs:\n  - name: a\n    run: echo a\n    change: '**/*'\n",
                "hooks:\n  close: 'echo {{filepath}}'\njobs:\n  - name: a\n    run: echo a\n    change: '**/*'\n",
            ] {
                fs::write(root.join(".watch.yaml"), invalid).unwrap();
                let status = Command::new(env!("CARGO_BIN_EXE_fzz"))
                    .current_dir(&root)
                    .arg("check")
                    .status()
                    .unwrap();
                assert!(!status.success(), "invalid close must fail check");
            }
            fs::remove_dir_all(root).unwrap();
        });
    }
}
