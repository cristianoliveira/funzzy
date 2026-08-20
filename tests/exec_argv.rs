//! Ad-hoc `exec` mode must preserve child argv end to end.
//!
//! `fzz exec -- PROGRAM ARG...` spawns PROGRAM directly with the exact
//! ARG... vector. Arguments are never joined and re-parsed through a shell,
//! so shell quoting/operators are irrelevant unless the caller explicitly
//! invokes a shell (e.g. `fzz exec -- sh -c '...'`).

use std::io::prelude::*;
use std::process::{Command, Stdio};

#[path = "./common/lib.rs"]
mod setup;

#[test]
fn test_it_preserves_argv_boundaries_when_piping_files() {
    let test_log_file = "test_it_preserves_argv_boundaries_when_piping_files.log";
    setup::with_output(test_log_file, |fzz_cmd, mut output_log, fixture| {
        let mut files = Command::new("find")
            .arg(".")
            .arg("-name")
            .arg("*.txt")
            .current_dir(fixture)
            .stdout(Stdio::piped())
            .spawn()
            .expect("failed to run find");

        // Two distinct argv elements: `a b` must reach `printf` as ONE
        // argument. A joined-and-reparsed command (`printf '<%s>\n' a b`)
        // would print `<a>` and `<b>` separately instead of `<a b>`.
        let mut child = fzz_cmd
            .args(["exec", "--", "printf", "<%s>\\n", "a b", "c"])
            .stdin(files.stdout.take().expect("failed to open stdin"))
            .spawn()
            .expect("Failed to spawn fzz");
        let _ = files.wait();

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

                output.contains("<a b>") && output.contains("<c>")
            },
            "argv boundaries were lost: output was:\n{}",
            output
        );
    });
}

#[test]
fn test_it_runs_shell_only_when_explicitly_invoked() {
    let test_log_file = "test_it_runs_shell_only_when_explicitly_invoked.log";
    setup::with_output(test_log_file, |fzz_cmd, mut output_log, fixture| {
        let mut files = Command::new("find")
            .arg(".")
            .arg("-name")
            .arg("*.txt")
            .current_dir(fixture)
            .stdout(Stdio::piped())
            .spawn()
            .expect("failed to run find");

        // `echo` receives the literal `a | b`; without a shell there is no
        // pipe operator, so the full string must be printed verbatim.
        let mut child = fzz_cmd
            .args(["exec", "--", "echo", "a | b"])
            .stdin(files.stdout.take().expect("failed to open stdin"))
            .spawn()
            .expect("Failed to spawn fzz");
        let _ = files.wait();

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

                output.contains("a | b")
            },
            "shell operators must not run implicitly: output was:\n{}",
            output
        );
    });
}

#[test]
fn test_it_invokes_shell_when_caller_asks_for_one() {
    let test_log_file = "test_it_invokes_shell_when_caller_asks_for_one.log";
    setup::with_output(test_log_file, |fzz_cmd, mut output_log, fixture| {
        let mut files = Command::new("find")
            .arg(".")
            .arg("-name")
            .arg("*.txt")
            .current_dir(fixture)
            .stdout(Stdio::piped())
            .spawn()
            .expect("failed to run find");

        // Explicit `sh -c` keeps shell semantics available on demand.
        let mut child = fzz_cmd
            .args(["exec", "--", "sh", "-c", "echo shell-ran"])
            .stdin(files.stdout.take().expect("failed to open stdin"))
            .spawn()
            .expect("Failed to spawn fzz");
        let _ = files.wait();

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

                output.contains("shell-ran")
            },
            "explicit shell invocation failed: output was:\n{}",
            output
        );
    });
}

#[test]
fn test_it_expands_templates_in_argv_elements() {
    let test_log_file = "test_it_expands_templates_in_argv_elements.log";
    setup::with_output(test_log_file, |fzz_cmd, mut output_log, fixture| {
        let mut files = Command::new("find")
            .arg(".")
            .arg("-name")
            .arg("*.txt")
            .current_dir(fixture)
            .stdout(Stdio::piped())
            .spawn()
            .expect("failed to run find");

        // The `{{filepath}}` template lives inside one argv element; it must
        // expand per changed path while the rest of the argv stays intact.
        let mut child = fzz_cmd
            .args(["exec", "--", "printf", "changed:%s", "{{filepath}}"])
            .stdin(files.stdout.take().expect("failed to open stdin"))
            .spawn()
            .expect("Failed to spawn fzz");
        let _ = files.wait();

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

                output.contains("changed:")
            },
            "template inside argv element must expand: output was:\n{}",
            output
        );
    });
}

#[test]
fn test_it_reports_child_failure_but_keeps_watching() {
    let test_log_file = "test_it_reports_child_failure_but_keeps_watching.log";
    setup::with_output(test_log_file, |fzz_cmd, mut output_log, fixture| {
        let mut files = Command::new("find")
            .arg(".")
            .arg("-name")
            .arg("*.txt")
            .current_dir(fixture)
            .stdout(Stdio::piped())
            .spawn()
            .expect("failed to run find");

        // `sh -c 'exit 3'` is an argv whose child exits non-zero. The run
        // must be reported as failed (child-side) and the watcher must stay
        // alive to keep watching.
        let mut child = fzz_cmd
            .args(["exec", "--", "sh", "-c", "exit 3"])
            .stdin(files.stdout.take().expect("failed to open stdin"))
            .spawn()
            .expect("Failed to spawn fzz");
        let _ = files.wait();

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

                output.contains("has failed with exit status: 3")
            },
            "child failure must be reported: output was:\n{}",
            output
        );

        // Watcher is still alive: trigger a change and expect a new run.
        write_to_file!(fixture.join("examples/workdir/another_ignored_file.foo"));
        write_to_file!(fixture.join("examples/workdir/trigger-watcher.txt"));

        let mut output_after = String::new();
        wait_until!(
            {
                output_log
                    .read_to_string(&mut output_after)
                    .expect("failed to read from file");

                output_after.split("has failed with exit status: 3").count() > 1
            },
            "watcher must keep running after a child failure: output was:\n{}",
            output_after
        );
    });
}
