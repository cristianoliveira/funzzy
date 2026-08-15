//! Process-group ownership contract tests (TASK-0030).
//!
//! Each task leads its own process group, so restart cancellation signals the
//! whole tree (shell + descendants) instead of only the direct child PID.
//! These tests prove no grandchild is orphaned by a restart.

use std::io::prelude::*;

#[path = "./common/lib.rs"]
mod setup;

/// Probes whether a process is still alive using `kill -0`.
fn alive(pid: u32) -> bool {
    std::process::Command::new("kill")
        .arg("-0")
        .arg(pid.to_string())
        .output()
        .map(|o| o.status.success())
        .unwrap_or(false)
}

/// A task that spawns a long-lived grandchild (`sleep`) and records its PID.
/// Before TASK-0030, cancelling the bash child left `sleep` orphaned because
/// only the direct PID was signaled. With process-group ownership, the group
/// signal reaches the grandchild too.
#[test]
fn ctrl_c_shuts_down_owned_groups_before_exit() {
    setup::with_output(
        "process_groups_ctrl_c.log",
        |fzz_cmd, mut output_log, fixture| {
            std::fs::write(
                fixture.join(".watch.yaml"),
                "- name: ctrl-c-grandchild
  run: \"bash -c 'sleep 30 & echo $! > ctrl-c-grandchild.pid; echo CTRL_C_READY; wait'\"
  change: 'trigger.txt'
  run_on_init: true
",
            )
            .expect("write fixture config");

            let mut child = fzz_cmd.arg("--restart").spawn().expect("spawn fzz");

            let mut output = String::new();
            wait_until!(
                {
                    output_log.read_to_string(&mut output).expect("read output");
                    output.contains("CTRL_C_READY")
                },
                "task never reported ready: {}",
                output
            );

            // The grandchild pid file is written by the background subshell
            // after CTRL_C_READY; under full-suite load a fixed sleep races
            // that write, so wait for the file deterministically instead.
            let pid_file = fixture.join("ctrl-c-grandchild.pid");
            wait_until!(
                {
                    let ready = std::fs::read_to_string(&pid_file)
                        .map(|content| !content.trim().is_empty())
                        .unwrap_or(false);
                    ready
                },
                "grandchild pid file never appeared at {}",
                pid_file.display()
            );
            let grandchild_pid: u32 = std::fs::read_to_string(&pid_file)
                .expect("grandchild pid file")
                .trim()
                .parse()
                .expect("pid");
            assert!(alive(grandchild_pid), "grandchild should start alive");

            let status = std::process::Command::new("kill")
                .arg("-INT")
                .arg(child.id().to_string())
                .status()
                .expect("send SIGINT");
            assert!(status.success(), "failed to send SIGINT");

            let deadline = std::time::Instant::now() + std::time::Duration::from_secs(10);
            let exit_status = loop {
                if let Some(status) = child.try_wait().expect("poll fzz") {
                    break status;
                }
                assert!(
                    std::time::Instant::now() < deadline,
                    "fzz did not exit after SIGINT"
                );
                std::thread::sleep(std::time::Duration::from_millis(20));
            };
            assert_eq!(
                exit_status.code(),
                Some(130),
                "SIGINT should preserve conventional exit code"
            );
            assert!(
                !alive(grandchild_pid),
                "Ctrl-C orphaned grandchild {}",
                grandchild_pid
            );
        },
    );
}

#[test]
fn restart_cancel_reaches_grandchildren() {
    setup::with_output(
        "process_groups_grandchildren.log",
        |fzz_cmd, mut output_log, fixture| {
            std::fs::write(
                fixture.join(".watch.yaml"),
                "- name: spawns-grandchild
  run: \"bash -c 'echo GRANDCHILD_READY; sleep 30 & echo $! > grandchild.pid; wait'\"
  change: 'trigger.txt'
  run_on_init: true
",
            )
            .expect("write fixture config");

            let mut child = fzz_cmd.arg("--restart").spawn().expect("spawn fzz");
            defer!({
                let _ = child.kill();
            });

            let mut output = String::new();
            wait_until!(
                {
                    output_log.read_to_string(&mut output).expect("read output");
                    output.contains("GRANDCHILD_READY")
                },
                "task never reported grandchild ready: {}",
                output
            );

            // Give the shell a moment to write the grandchild PID file.
            std::thread::sleep(std::time::Duration::from_millis(300));
            let grandchild_pid: u32 = std::fs::read_to_string(fixture.join("grandchild.pid"))
                .expect("grandchild.pid")
                .trim()
                .parse()
                .expect("pid");

            // Sanity: the grandchild is alive before cancellation.
            assert!(
                alive(grandchild_pid),
                "grandchild should be alive before cancel"
            );

            // Trigger a restart: the change cancels the running task.
            std::fs::write(fixture.join("trigger.txt"), "restart").expect("write trigger");

            // Wait for the replacement generation to run and cancel the prior one.
            wait_until!(
                {
                    output_log.read_to_string(&mut output).expect("read output");
                    // The restarted task prints GRANDCHILD_READY a second time.
                    output.matches("GRANDCHILD_READY").count() >= 2
                },
                "restart never re-ran the task: {}",
                output
            );

            // The first generation's grandchild must have been reaped by the
            // group signal, not orphaned. Allow time for the grace period.
            wait_until!(
                !alive(grandchild_pid),
                "grandchild {} was orphaned by restart cancel",
                grandchild_pid
            );
        },
    );
}
