use crate::logging;
use crate::stdout;
use nix::sys::signal::{self, Signal};
use nix::unistd::Pid;
use std::io::{BufRead, BufReader, Write};
use std::process::{Child, Command, ExitStatus, Stdio};
use std::thread;
use std::time::{Duration, Instant};

/// Keeps track of the threads that forward the child's stdout/stderr so they
/// can be joined once the child process exits.
struct ForwardHandles {
    stdout: Option<thread::JoinHandle<()>>,
    stderr: Option<thread::JoinHandle<()>>,
}

impl ForwardHandles {
    fn new() -> Self {
        Self {
            stdout: None,
            stderr: None,
        }
    }

    fn join(&mut self) {
        if let Some(handle) = self.stdout.take() {
            let _ = handle.join();
        }

        if let Some(handle) = self.stderr.take() {
            let _ = handle.join();
        }
    }

    fn discard(&mut self) {
        self.stdout.take();
        self.stderr.take();
    }
}

/// Wraps a [`Child`] whose stdout/stderr are being forwarded by background
/// threads. The wrapper makes sure those threads are joined once the child
/// finishes so no output is lost and the process shuts down cleanly.
pub struct LoggedChild {
    child: Child,
    forward_handles: ForwardHandles,
    has_finished: bool,
}

/// Outcome of a graceful process-group shutdown (TASK-0030).
#[derive(Debug)]
pub enum ShutdownOutcome {
    /// The child had already exited before any signal was sent.
    AlreadyExited(ExitStatus),
    /// The group terminated after the initial signal, within the grace period.
    Terminated(ExitStatus),
    /// The grace period elapsed; the group was force-killed (`SIGKILL`) and
    /// reaped. `status` is `None` only if the final reap failed (exceptional).
    Escalated { status: Option<ExitStatus> },
}

impl LoggedChild {
    fn new(mut child: Child) -> Self {
        let forward_handles = forward_child_output(&mut child);
        Self {
            child,
            forward_handles,
            has_finished: false,
        }
    }

    pub fn id(&self) -> u32 {
        self.child.id()
    }

    pub fn try_wait(&mut self) -> std::io::Result<Option<ExitStatus>> {
        let result = self.child.try_wait()?;

        if result.is_some() {
            self.join_forwarding_threads();
        }

        Ok(result)
    }

    pub fn wait(&mut self) -> std::io::Result<ExitStatus> {
        let status = self.child.wait();

        match status {
            Ok(ref _status) => {
                self.join_forwarding_threads();
            }
            Err(_) => {
                self.forward_handles.discard();
            }
        }

        status
    }

    #[allow(dead_code)]
    pub fn kill(&mut self) -> std::io::Result<()> {
        self.child.kill()
    }

    fn join_forwarding_threads(&mut self) {
        if !self.has_finished {
            self.forward_handles.join();
            self.has_finished = true;
        }
    }

    /// Gracefully shuts the owned process group down: sends `signal` to the
    /// whole group (shell + descendants), waits up to `grace` for exit, then
    /// escalates to `SIGKILL` on the group and reaps. Forwarding threads are
    /// always joined before returning so no output is lost and no zombie is
    /// left behind (TASK-0030).
    pub fn shutdown(&mut self, signal: Signal, grace: Duration, verbose: bool) -> ShutdownOutcome {
        // 1. Already exited?
        if let Ok(Some(status)) = self.child.try_wait() {
            self.join_forwarding_threads();
            return ShutdownOutcome::AlreadyExited(status);
        }

        let pgid = self.child.id() as i32;
        stdout::verbose(
            &format!(
                "---- signalling process group -{} with {:?} ----",
                pgid, signal
            ),
            verbose,
        );

        // 2. Signal the whole process group (negative pid).
        let _ = signal::kill(Pid::from_raw(-pgid), signal);

        // 3. Wait up to `grace` for the group to terminate.
        let deadline = Instant::now() + grace;
        loop {
            if let Ok(Some(status)) = self.child.try_wait() {
                self.join_forwarding_threads();
                return ShutdownOutcome::Terminated(status);
            }
            if Instant::now() >= deadline {
                break;
            }
            thread::sleep(Duration::from_millis(20));
        }

        // 4. Grace elapsed: escalate to SIGKILL on the group and reap.
        stdout::verbose(
            &format!(
                "---- grace of {:?} elapsed; force-killing process group -{} ----",
                grace, pgid
            ),
            verbose,
        );
        let _ = signal::kill(Pid::from_raw(-pgid), Signal::SIGKILL);
        let status = self.child.wait().ok();
        self.join_forwarding_threads();
        ShutdownOutcome::Escalated { status }
    }
}

impl Drop for LoggedChild {
    fn drop(&mut self) {
        if !self.has_finished {
            match self.child.try_wait() {
                Ok(Some(_)) => self.join_forwarding_threads(),
                _ => {
                    // Last-resort owner cleanup: a dropped live handle must
                    // not orphan its process group. Normal executor shutdown
                    // calls `shutdown` explicitly with the configured grace;
                    // Drop uses a short deterministic grace, then SIGKILL.
                    let _ = self.shutdown(Signal::SIGTERM, Duration::from_millis(100), false);
                }
            }
        }
        crate::process_owner::unregister(self.child.id() as i32);
    }
}

pub fn execute(command: &String) -> Result<(), String> {
    println!();
    logging::log_line("");
    stdout::info(&format!("{} \n", String::from(command)));

    let mut cmd = prepare_command(command);

    run_to_completion(&mut cmd, command)
}

/// Executes an exact argv (program plus arguments) directly, without a
/// shell. Spawn failures are Funzzy-side errors; non-zero exits are child
/// failures. Used by ad-hoc `exec` mode so argv never gets joined/re-parsed.
pub fn execute_argv(argv: &[String]) -> Result<(), String> {
    let display = argv.join(" ");
    println!();
    logging::log_line("");
    stdout::info(&format!("{} \n", display));

    let mut cmd = prepare_argv_command(argv);

    run_to_completion(&mut cmd, &display)
}

fn run_to_completion(cmd: &mut Command, display: &str) -> Result<(), String> {
    if logging::is_enabled() {
        cmd.stdout(Stdio::piped());
        cmd.stderr(Stdio::piped());

        let mut child = cmd
            .spawn()
            .map_err(|error| format!("Command {} has errored with {}", display, error))?;

        let mut handles = forward_child_output(&mut child);

        match child.wait() {
            Ok(status) if status.success() => {
                handles.join();
                Ok(())
            }
            Ok(status) => {
                handles.join();
                Err(format!("Command {} has failed with {}", display, status))
            }
            Err(error) => {
                handles.discard();
                Err(format!("Command {} has errored with {}", display, error))
            }
        }
    } else {
        match cmd.status() {
            Err(error) => Err(format!("Command {} has errored with {}", display, error)),
            Ok(status) => {
                if status.success() {
                    Ok(())
                } else {
                    Err(format!("Command {} has failed with {}", display, status))
                }
            }
        }
    }
}

pub fn spawn(command: &String) -> Result<LoggedChild, String> {
    println!();
    logging::log_line("");
    stdout::info(&format!("{} \n", String::from(command)));

    let mut cmd = prepare_command(command);

    spawn_configured(&mut cmd, command)
}

/// Spawns an exact argv (program plus arguments) directly, without a shell.
/// Returns a child that can be cancelled; used by ad-hoc `exec` mode.
pub fn spawn_argv(argv: &[String]) -> Result<LoggedChild, String> {
    let display = argv.join(" ");
    println!();
    logging::log_line("");
    stdout::info(&format!("{} \n", display));

    let mut cmd = prepare_argv_command(argv);

    spawn_configured(&mut cmd, &display)
}

fn spawn_configured(cmd: &mut Command, display: &str) -> Result<LoggedChild, String> {
    if logging::is_enabled() {
        cmd.stdout(Stdio::piped());
        cmd.stderr(Stdio::piped());
    }

    // Each task leads its own process group so cancellation can signal the
    // whole tree (shell + descendants) without touching the funzzy process
    // group. Done in pre_exec to avoid the parent/child exec race. Requires
    // an explicit SIGINT route (see app.rs) since these groups no longer
    // share funzzy's foreground group. (TASK-0030)
    #[cfg(unix)]
    unsafe {
        use std::os::unix::process::CommandExt;
        cmd.pre_exec(|| {
            use nix::sys::signal::{sigprocmask, SigSet, SigmaskHow};

            nix::unistd::setpgid(nix::unistd::Pid::from_raw(0), nix::unistd::Pid::from_raw(0))
                .map_err(|err| std::io::Error::from_raw_os_error(err as i32))?;

            // The non-block parent blocks SIGINT/SIGTERM for its sigwait
            // thread. Signal masks are inherited across fork/exec, so reset
            // them in the child; otherwise graceful group SIGTERM would stay
            // pending until the 5s grace elapsed and every cancel would
            // unnecessarily escalate to SIGKILL.
            let mut signals = SigSet::empty();
            signals.add(Signal::SIGINT);
            signals.add(Signal::SIGTERM);
            sigprocmask(SigmaskHow::SIG_UNBLOCK, Some(&signals), None)
                .map_err(|err| std::io::Error::from_raw_os_error(err as i32))?;
            Ok(())
        });
    }

    match cmd.spawn() {
        Ok(child) => {
            // The child leads its own group (pgid == pid). Track it so every
            // shutdown path can reach the whole task tree (TASK-0030).
            let pid = child.id() as i32;
            crate::process_owner::register(pid);
            Ok(LoggedChild::new(child))
        }
        Err(error) => Err(format!("Command {} has errored with {}", display, error)),
    }
}

#[test]
fn it_spawn_a_command_returning_a_child_ref() {
    let result = match spawn(&String::from("echo 'foo'")) {
        Ok(mut child) => child.wait().expect("fail to wait"),
        Err(err) => panic!("{:?}", err),
    };

    assert_eq!(format!("{}", result), "exit status: 0")
}

#[test]
fn graceful_shutdown_escalates_when_group_ignores_sigterm() {
    let ready =
        std::env::temp_dir().join(format!("funzzy-term-ignore-ready-{}", std::process::id()));
    let _ = std::fs::remove_file(&ready);
    let command = format!(
        "bash -c 'trap \"\" TERM; touch {}; while true; do sleep 1; done'",
        ready.display()
    );
    let mut child = spawn(&command).expect("spawn TERM-ignoring group");

    let deadline = Instant::now() + Duration::from_secs(2);
    while !ready.exists() && Instant::now() < deadline {
        thread::sleep(Duration::from_millis(10));
    }
    assert!(ready.exists(), "child never installed TERM trap");

    let outcome = child.shutdown(Signal::SIGTERM, Duration::from_millis(50), false);
    let _ = std::fs::remove_file(&ready);
    assert!(
        matches!(outcome, ShutdownOutcome::Escalated { .. }),
        "TERM-ignoring group must escalate: {:?}",
        outcome
    );
}

#[test]
fn repeated_shutdown_is_safe() {
    let mut child = spawn(&"sleep 30".to_owned()).expect("spawn sleep");
    let _ = child.shutdown(Signal::SIGTERM, Duration::from_millis(200), false);
    let second = child.shutdown(Signal::SIGTERM, Duration::from_millis(20), false);
    assert!(
        matches!(second, ShutdownOutcome::AlreadyExited(_)),
        "second shutdown must be an idempotent no-op: {:?}",
        second
    );
}

#[test]
fn dropping_owner_terminates_its_process_group() {
    let child = spawn(&"sleep 30".to_owned()).expect("spawn sleep");
    let pgid = child.id() as i32;
    drop(child);

    let deadline = Instant::now() + Duration::from_secs(2);
    while Instant::now() < deadline {
        if matches!(
            signal::kill(Pid::from_raw(-pgid), None),
            Err(nix::errno::Errno::ESRCH)
        ) {
            return;
        }
        thread::sleep(Duration::from_millis(20));
    }
    panic!("dropped owner left process group {} alive", pgid);
}

#[test]
fn it_executes_argv_directly_without_a_shell() {
    // `printf '<%s>' 'a b'` must print one arg `a b`; if the argv were joined
    // and re-parsed through a shell, `a b` would become two arguments.
    let result = execute_argv(&["printf".to_owned(), "<%s>".to_owned(), "a b".to_owned()]);
    assert!(
        result.is_ok(),
        "argv execution should succeed: {:?}",
        result
    );
}

#[test]
fn it_reports_child_failure_for_non_zero_argv_exit() {
    let result = execute_argv(&["sh".to_owned(), "-c".to_owned(), "exit 3".to_owned()]);
    let err = result.expect_err("non-zero exit must fail");
    assert!(
        err.contains("Command sh -c exit 3 has failed with"),
        "unexpected error: {}",
        err
    );
}

#[test]
fn it_reports_spawn_failure_as_funzzy_side_error() {
    let result = execute_argv(&["definitely-not-a-real-program-xyz".to_owned()]);
    let err = result.expect_err("missing program must fail to start");
    assert!(
        err.contains("has errored with"),
        "spawn failure must be reported: {}",
        err
    );
}

#[test]
fn it_executes_a_command() {
    let result = match execute(&String::from("echo 'foo'")) {
        Ok(_) => true,
        Err(err) => panic!("{:?}", err),
    };

    assert!(result)
}

fn prepare_command(command: &String) -> Command {
    let shell = std::env::var("SHELL").unwrap_or(String::from("/bin/sh"));
    let mut cmd = Command::new(shell);
    cmd.arg("-c").arg(command);
    cmd
}

fn prepare_argv_command(argv: &[String]) -> Command {
    let mut cmd = Command::new(&argv[0]);
    cmd.args(&argv[1..]);
    cmd
}

fn forward_child_output(child: &mut Child) -> ForwardHandles {
    let mut handles = ForwardHandles::new();

    if let Some(stdout) = child.stdout.take() {
        handles.stdout = Some(spawn_forwarding_thread(stdout, false));
    }

    if let Some(stderr) = child.stderr.take() {
        handles.stderr = Some(spawn_forwarding_thread(stderr, true));
    }

    handles
}

fn spawn_forwarding_thread<R: std::io::Read + Send + 'static>(
    reader: R,
    is_stderr: bool,
) -> thread::JoinHandle<()> {
    thread::spawn(move || {
        let mut reader = BufReader::new(reader);
        let mut line = String::new();

        loop {
            line.clear();

            match reader.read_line(&mut line) {
                Ok(0) => break,
                Ok(_) => {
                    if is_stderr {
                        eprint!("{}", line);
                        let _ = std::io::stderr().flush();
                    } else {
                        print!("{}", line);
                        let _ = std::io::stdout().flush();
                    }

                    logging::log_plain(&line);
                }
                Err(_) => break,
            }
        }
    })
}
