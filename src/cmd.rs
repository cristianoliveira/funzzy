use crate::diagnostics;
use crate::logging;
use crate::plan::TaskContext;
use crate::stdout;
use nix::sys::signal::{self, Signal};
use nix::unistd::Pid;
use std::io::{BufRead, BufReader, Write};
use std::process::{Child, Command, ExitStatus, Stdio};
use std::sync::{Arc, Mutex};
use std::thread;
use std::time::{Duration, Instant};

/// Per-stream byte bound for retained task output (contract §6). The tail up
/// to this size is kept; anything older is evicted and marked truncated.
pub const CAPTURE_STREAM_BYTES: usize = 64 * 1024;

/// Bounded per-stream capture buffer (contract §6): keeps the newest bytes up
/// to [`CAPTURE_STREAM_BYTES`], always marks truncation, and reports the
/// total observed size. Never infers secrets — raw bytes only.
#[derive(Clone, Debug)]
pub struct CaptureBuffer {
    bound: usize,
    bytes: Vec<u8>,
    observed: u64,
    truncated: bool,
}

impl CaptureBuffer {
    fn new(bound: usize) -> Self {
        Self {
            bound,
            bytes: Vec::new(),
            observed: 0,
            truncated: false,
        }
    }

    fn append(&mut self, chunk: &[u8]) {
        self.observed += chunk.len() as u64;
        self.bytes.extend_from_slice(chunk);
        let over = self.bytes.len().saturating_sub(self.bound);
        if over > 0 {
            self.bytes.drain(..over);
            self.truncated = true;
        }
    }

    /// Final retained bytes (raw, lossy-rendered at retrieval).
    pub fn bytes(&self) -> &[u8] {
        &self.bytes
    }

    pub fn observed_bytes(&self) -> u64 {
        self.observed
    }

    pub fn retained_bytes(&self) -> u64 {
        self.bytes.len() as u64
    }

    pub fn truncated(&self) -> bool {
        self.truncated
    }
}

/// Final per-stream capture of one task (contract §6).
pub struct CaptureData {
    pub stdout: CaptureBuffer,
    pub stderr: CaptureBuffer,
}

impl Default for CaptureData {
    fn default() -> Self {
        Self {
            stdout: CaptureBuffer::new(CAPTURE_STREAM_BYTES),
            stderr: CaptureBuffer::new(CAPTURE_STREAM_BYTES),
        }
    }
}

impl CaptureData {
    fn new(stdout_bound: usize, stderr_bound: usize) -> Self {
        Self {
            stdout: CaptureBuffer::new(stdout_bound),
            stderr: CaptureBuffer::new(stderr_bound),
        }
    }
}

/// Shared capture sink fed by the child's forwarding threads (single read,
/// multiple consumers: live print + log file + bounded capture).
pub struct CaptureHandle {
    data: Mutex<CaptureData>,
}

impl CaptureHandle {
    pub fn new() -> Self {
        Self {
            data: Mutex::new(CaptureData::new(CAPTURE_STREAM_BYTES, CAPTURE_STREAM_BYTES)),
        }
    }

    pub fn append(&self, bytes: &[u8], is_stderr: bool) {
        let mut data = self.data.lock().unwrap();
        if is_stderr {
            data.stderr.append(bytes);
        } else {
            data.stdout.append(bytes);
        }
    }

    /// Extracts the final captured data (cloned; bounded by the stream caps).
    pub fn finish(&self) -> CaptureData {
        let data = self.data.lock().unwrap();
        CaptureData {
            stdout: CaptureBuffer {
                bound: data.stdout.bound,
                bytes: data.stdout.bytes.clone(),
                observed: data.stdout.observed,
                truncated: data.stdout.truncated,
            },
            stderr: CaptureBuffer {
                bound: data.stderr.bound,
                bytes: data.stderr.bytes.clone(),
                observed: data.stderr.observed,
                truncated: data.stderr.truncated,
            },
        }
    }
}

impl Default for CaptureHandle {
    fn default() -> Self {
        Self::new()
    }
}

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
    fn new(
        mut child: Child,
        capture: Option<Arc<CaptureHandle>>,
        label: Option<String>,
        quiet: bool,
    ) -> Self {
        let forward_handles = forward_child_output(&mut child, capture, label, quiet);
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
        if verbose {
            diagnostics::debug(&diagnostics::Record {
                decision: Some("cancel"),
                note: Some(format!(
                    "signalling process group -{} with {:?}",
                    pgid, signal
                )),
                ..Default::default()
            });
        }

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
        if verbose {
            diagnostics::debug(&diagnostics::Record {
                decision: Some("cancel"),
                note: Some(format!(
                    "grace of {:?} elapsed; force-killing process group -{}",
                    grace, pgid
                )),
                ..Default::default()
            });
        }
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

        let mut handles = forward_child_output(&mut child, None, None, false);

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
    spawn_in(command, &TaskContext::default())
}

pub fn spawn_in(command: &String, context: &TaskContext) -> Result<LoggedChild, String> {
    spawn_in_with_capture(command, context, None, None)
}

/// Like [`spawn_in_with_capture`] with a quiet flag (TASK-0041): when quiet,
/// live output is suppressed (capture still keeps raw bytes) so policies
/// like quiet/capture/show-on-failure can hold output until reveal.
pub fn spawn_in_with_capture_quiet(
    command: &String,
    context: &TaskContext,
    capture: Option<Arc<CaptureHandle>>,
    label: Option<String>,
    quiet: bool,
) -> Result<LoggedChild, String> {
    println!();
    logging::log_line("");
    stdout::info(&format!("{} \n", String::from(command)));

    let mut cmd = prepare_command(command);
    apply_context(&mut cmd, context);

    spawn_configured(&mut cmd, command, capture, label, quiet)
}

/// Spawns with an optional bounded output capture (TASK-0045, contract §6).
/// The capture shares the child's pipe reads: live forwarding and the log
/// file keep working exactly as before, with one read, multiple sinks.
/// `label` (TASK-0028) prefixes every live line with `[label] ` so parallel
/// tasks keep task identity; raw capture bytes are never prefixed.
pub fn spawn_in_with_capture(
    command: &String,
    context: &TaskContext,
    capture: Option<Arc<CaptureHandle>>,
    label: Option<String>,
) -> Result<LoggedChild, String> {
    println!();
    logging::log_line("");
    stdout::info(&format!("{} \n", String::from(command)));

    let mut cmd = prepare_command(command);
    apply_context(&mut cmd, context);

    spawn_configured(&mut cmd, command, capture, label, false)
}

/// Spawns an exact argv (program plus arguments) directly, without a shell.
/// Returns a child that can be cancelled; used by ad-hoc `exec` mode.
pub fn spawn_argv(argv: &[String]) -> Result<LoggedChild, String> {
    spawn_argv_in(argv, &TaskContext::default())
}

pub fn spawn_argv_in(argv: &[String], context: &TaskContext) -> Result<LoggedChild, String> {
    let display = argv.join(" ");
    println!();
    logging::log_line("");
    stdout::info(&format!("{} \n", display));

    let mut cmd = prepare_argv_command(argv);
    apply_context(&mut cmd, context);

    spawn_configured(&mut cmd, &display, None, None, false)
}

fn apply_context(command: &mut Command, context: &TaskContext) {
    if let Some(cwd) = &context.cwd {
        command.current_dir(cwd);
    }
    command.envs(&context.environment);
}

fn spawn_configured(
    cmd: &mut Command,
    display: &str,
    capture: Option<Arc<CaptureHandle>>,
    label: Option<String>,
    quiet: bool,
) -> Result<LoggedChild, String> {
    // Pipe child output whenever we need to forward it: logging, bounded
    // capture, quiet suppression (TASK-0041), or live task attribution
    // (TASK-0028). Attribution requires the forwarding thread, so
    // parallel-group tasks always pipe even without `--log-file`; serial
    // tasks without logging/capture/quiet keep inherited stdout passthrough.
    if logging::is_enabled() || capture.is_some() || label.is_some() || quiet {
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
            Ok(LoggedChild::new(child, capture, label, quiet))
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

fn forward_child_output(
    child: &mut Child,
    capture: Option<Arc<CaptureHandle>>,
    label: Option<String>,
    quiet: bool,
) -> ForwardHandles {
    let mut handles = ForwardHandles::new();

    if let Some(stdout) = child.stdout.take() {
        handles.stdout = Some(spawn_forwarding_thread(
            stdout,
            false,
            capture.clone(),
            label.clone(),
            quiet,
        ));
    }

    if let Some(stderr) = child.stderr.take() {
        handles.stderr = Some(spawn_forwarding_thread(stderr, true, capture, label, quiet));
    }

    handles
}

/// Byte-safe, line-atomic child output forwarding (TASK-0028, contract §6):
/// reads raw bytes until each newline, so invalid UTF-8 never corrupts or
/// drops output (rendered lossy), partial final lines are still emitted, and
/// one whole line is written per call so concurrent tasks cannot interleave
/// mid-line. When `label` is set, every live line is prefixed with `[label] `
/// so parallel tasks keep task identity; the capture always keeps the raw
/// bytes, never the rendered prefix.
/// Renders one raw child-output chunk (a full line, including its trailing
/// newline, or a partial final line) for live forwarding: lossy UTF-8 so
/// binary output never drops the stream, with an optional `[label] ` prefix
/// for parallel-task attribution (TASK-0028). Pure and testable.
fn render_live_line(raw: &[u8], label: Option<&str>) -> String {
    let rendered = String::from_utf8_lossy(raw);
    match label {
        Some(label) => format!("[{}] {}", label, rendered),
        None => rendered.into_owned(),
    }
}

fn spawn_forwarding_thread<R: std::io::Read + Send + 'static>(
    reader: R,
    is_stderr: bool,
    capture: Option<Arc<CaptureHandle>>,
    label: Option<String>,
    quiet: bool,
) -> thread::JoinHandle<()> {
    thread::spawn(move || {
        let mut reader = BufReader::new(reader);
        let mut line: Vec<u8> = Vec::new();

        loop {
            line.clear();

            match reader.read_until(b'\n', &mut line) {
                Ok(0) => break,
                Ok(_) => {
                    // Lossy render: binary or invalid UTF-8 output still
                    // reaches the console instead of silently dropping the
                    // rest of the stream.
                    let attributed = render_live_line(&line, label.as_deref());
                    // TASK-0041: quiet/capture/show-on-failure suppress the
                    // live stream; the capture still keeps raw bytes so the
                    // output is retrievable and revealable on failure.
                    if !quiet {
                        if is_stderr {
                            eprint!("{}", attributed);
                            let _ = std::io::stderr().flush();
                        } else {
                            print!("{}", attributed);
                            let _ = std::io::stdout().flush();
                        }
                    }
                    if !quiet {
                        logging::log_plain(&attributed);
                    }
                    if let Some(capture) = &capture {
                        // Raw bytes: no secret inference, no UTF-8 validation
                        // here (retrieval renders lossy), and no prefix.
                        capture.append(&line, is_stderr);
                    }
                }
                Err(_) => break,
            }
        }
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn render_live_line_preserves_plain_output_byte_for_byte() {
        assert_eq!(render_live_line(b"hello\n", None), "hello\n");
        assert_eq!(render_live_line(b"", None), "");
        assert_eq!(
            render_live_line(b"partial without newline", None),
            "partial without newline"
        );
    }

    #[test]
    fn render_live_line_prefixes_label_on_complete_and_partial_lines() {
        assert_eq!(
            render_live_line(b"line one\n", Some("lint")),
            "[lint] line one\n"
        );
        assert_eq!(
            render_live_line(b"trailing partial", Some("test")),
            "[test] trailing partial"
        );
    }

    #[test]
    fn render_live_line_is_lossy_for_non_utf8_but_never_drops_bytes() {
        // 0xFF is invalid UTF-8: rendered lossily (never kills the stream),
        // and every byte still appears in the rendered line.
        let rendered = render_live_line(b"ok \xff done\n", Some("check"));
        assert!(rendered.contains("[check] ok "), "rendered: {rendered}");
        assert!(rendered.contains(" done\n"), "rendered: {rendered}");
    }

    #[test]
    fn render_live_line_keeps_one_line_atomic_under_attribution() {
        // The prefix must not split a line: one whole line in, one whole
        // attributed line out, with the newline preserved at the end.
        let line = b"interleaved output stays whole\n";
        let rendered = render_live_line(line, Some("task a"));
        assert_eq!(rendered, "[task a] interleaved output stays whole\n");
        assert_eq!(rendered.matches('\n').count(), 1);
    }
}
