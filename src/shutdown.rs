//! One idempotent watcher shutdown coordinator (TASK-0101).
//!
//! Signal handlers only request shutdown through atomics/self-pipe. Normal
//! Rust control flow calls [`ShutdownCoordinator::finish`], which freezes the
//! first reason/exit code, reaps owned work, retires resources, runs the
//! latest committed close hook once, and reports completion to concurrent
//! callers. Finite commands never receive this coordinator.

use crate::config::SessionHooks;
use crate::diagnostics;
use crate::executor::{ProcessRunner, SystemProcessRunner};
use crate::plan::TaskContext;
use crate::rules::CommandLine;
use std::path::PathBuf;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Condvar, Mutex};
use std::time::{Duration, Instant};

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum ShutdownReason {
    Normal,
    Signal { name: &'static str, exit_code: i32 },
    FatalConfig { detail: String, exit_code: i32 },
    Operational { detail: String, exit_code: i32 },
}

impl ShutdownReason {
    pub fn exit_code(&self) -> i32 {
        match self {
            ShutdownReason::Normal => 0,
            ShutdownReason::Signal { exit_code, .. }
            | ShutdownReason::FatalConfig { exit_code, .. }
            | ShutdownReason::Operational { exit_code, .. } => *exit_code,
        }
    }

    pub fn label(&self) -> String {
        match self {
            ShutdownReason::Normal => "normal".to_owned(),
            ShutdownReason::Signal { name, .. } => (*name).to_owned(),
            ShutdownReason::FatalConfig { detail, .. } => format!("configInvalid: {detail}"),
            ShutdownReason::Operational { detail, .. } => format!("operational: {detail}"),
        }
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum CloseHookOutcome {
    NotConfigured,
    SkippedBeforeReady,
    Passed,
    Failed(String),
    TimedOut,
    Cancelled,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ShutdownCompletion {
    pub reason: ShutdownReason,
    pub hook: CloseHookOutcome,
}

pub trait ProcessReaper: Send + Sync {
    fn reap(&self, verbose: bool);
}

pub struct SystemProcessReaper;

impl ProcessReaper for SystemProcessReaper {
    fn reap(&self, verbose: bool) {
        let (signal, grace) = crate::process_owner::shutdown_policy();
        let _ = crate::process_owner::shutdown_all(signal, grace, verbose);
    }
}

#[derive(Clone)]
struct Requested {
    reason: ShutdownReason,
    hook: Option<String>,
}

enum Phase {
    Running,
    Requested(Requested),
    Finishing,
    Finished(ShutdownCompletion),
}

struct State {
    phase: Phase,
    ready: bool,
    committed_hooks: SessionHooks,
    cleanup_paths: Vec<PathBuf>,
}

pub struct ShutdownCoordinator {
    root: PathBuf,
    verbose: bool,
    runner: Arc<dyn ProcessRunner>,
    reaper: Arc<dyn ProcessReaper>,
    hook_timeout: Duration,
    state: Mutex<State>,
    completed: Condvar,
    requested: Arc<AtomicBool>,
    /// Exactly-once reap with completion visibility (TASK-0162): the signal
    /// thread claims and starts the reap, and `finish` blocks until the reap
    /// COMPLETED. Without the completion side, `process::exit` on the main
    /// thread could preempt the reaper mid-grace-loop and skip its SIGKILL
    /// escalation, orphaning TERM-ignoring service groups.
    reap: Mutex<ReapPhase>,
    reap_completed: Condvar,
    accelerate: AtomicBool,
}

#[derive(Clone, Copy, PartialEq, Eq)]
enum ReapPhase {
    Idle,
    Running,
    Done,
}

impl ShutdownCoordinator {
    pub fn system(root: PathBuf, hooks: SessionHooks, verbose: bool) -> Arc<Self> {
        let (_, grace) = crate::process_owner::shutdown_policy();
        Arc::new(Self::new(
            root,
            hooks,
            verbose,
            Arc::new(SystemProcessRunner),
            Arc::new(SystemProcessReaper),
            grace,
        ))
    }

    pub fn new(
        root: PathBuf,
        hooks: SessionHooks,
        verbose: bool,
        runner: Arc<dyn ProcessRunner>,
        reaper: Arc<dyn ProcessReaper>,
        hook_timeout: Duration,
    ) -> Self {
        Self {
            root,
            verbose,
            runner,
            reaper,
            hook_timeout,
            state: Mutex::new(State {
                phase: Phase::Running,
                ready: false,
                committed_hooks: hooks,
                cleanup_paths: Vec::new(),
            }),
            completed: Condvar::new(),
            requested: Arc::new(AtomicBool::new(false)),
            reap: Mutex::new(ReapPhase::Idle),
            reap_completed: Condvar::new(),
            accelerate: AtomicBool::new(false),
        }
    }

    /// Readiness is truthful only after filesystem watches and the control
    /// surface are registered. Startup failure before this call skips close.
    pub fn mark_ready(&self) {
        let mut state = self.state.lock().expect("shutdown mutex poisoned");
        if matches!(state.phase, Phase::Running) {
            state.ready = true;
        }
    }

    /// Valid reload commit replaces the future close hook atomically. Once a
    /// shutdown reason is claimed, later candidates cannot replace its snapshot.
    pub fn update_hooks(&self, hooks: SessionHooks) {
        let mut state = self.state.lock().expect("shutdown mutex poisoned");
        if matches!(state.phase, Phase::Running) {
            state.committed_hooks = hooks;
        }
    }

    pub fn set_cleanup_paths(&self, paths: Vec<PathBuf>) {
        self.state
            .lock()
            .expect("shutdown mutex poisoned")
            .cleanup_paths = paths;
    }

    /// First request freezes reason, exit code, readiness, and committed hook.
    /// Later requests never change them; a repeated signal asks an in-flight
    /// hook to stop promptly.
    pub fn request(&self, reason: ShutdownReason) -> bool {
        let first = {
            let mut state = self.state.lock().expect("shutdown mutex poisoned");
            if matches!(state.phase, Phase::Running) {
                let hook = if state.ready {
                    state.committed_hooks.close.clone()
                } else {
                    None
                };
                state.phase = Phase::Requested(Requested { reason, hook });
                self.requested.store(true, Ordering::SeqCst);
                true
            } else {
                if matches!(reason, ShutdownReason::Signal { .. }) {
                    self.accelerate.store(true, Ordering::SeqCst);
                }
                false
            }
        };
        if first {
            // Blocking run-on-init work cannot return to composition-root
            // cleanup until its owned process group is stopped. The shared
            // coordinator therefore performs this idempotent quiesce step on
            // first request; `finish` observes it and never reaps twice.
            self.reap_once();
        }
        first
    }

    /// Starts the shared reap exactly once; returns immediately when a reap
    /// was already started. The caller cannot assume completion — use
    /// [`ShutdownCoordinator::reap_wait`] when the process must not exit
    /// before every owned group is force-killed and reaped.
    fn reap_once(&self) {
        if self.claim_reap() {
            self.reaper.reap(self.verbose);
            self.finish_reap();
        }
    }

    fn claim_reap(&self) -> bool {
        let mut phase = self.reap.lock().expect("reap mutex poisoned");
        if *phase == ReapPhase::Idle {
            *phase = ReapPhase::Running;
            true
        } else {
            false
        }
    }

    fn finish_reap(&self) {
        let mut phase = self.reap.lock().expect("reap mutex poisoned");
        *phase = ReapPhase::Done;
        drop(phase);
        self.reap_completed.notify_all();
    }

    /// Ensures the shared reap runs and has COMPLETED before returning:
    /// starts it when idle (synchronously) and otherwise blocks on the
    /// in-flight reap started by the signal thread. This is the close-boundary
    /// guarantee that shutdown escalation cannot be preempted by exit.
    fn reap_wait(&self) {
        if self.claim_reap() {
            self.reaper.reap(self.verbose);
            self.finish_reap();
            return;
        }
        let mut phase = self.reap.lock().expect("reap mutex poisoned");
        while *phase == ReapPhase::Running {
            phase = self
                .reap_completed
                .wait(phase)
                .expect("reap condvar poisoned");
        }
    }

    pub fn is_requested(&self) -> bool {
        self.requested.load(Ordering::SeqCst)
    }

    /// Token-light wake flag for filesystem backends; policy stays here.
    pub fn requested_flag(&self) -> Arc<AtomicBool> {
        Arc::clone(&self.requested)
    }

    /// Completes the one shared close sequence. Concurrent callers wait for
    /// and receive the same immutable completion.
    pub fn finish(&self) -> ShutdownCompletion {
        let (requested, cleanup_paths, skipped_before_ready) = {
            let mut state = self.state.lock().expect("shutdown mutex poisoned");
            loop {
                match &state.phase {
                    Phase::Running => {
                        let hook = if state.ready {
                            state.committed_hooks.close.clone()
                        } else {
                            None
                        };
                        let skipped = !state.ready && state.committed_hooks.close.is_some();
                        let requested = Requested {
                            reason: ShutdownReason::Normal,
                            hook,
                        };
                        let paths = state.cleanup_paths.clone();
                        state.phase = Phase::Finishing;
                        self.requested.store(true, Ordering::SeqCst);
                        break (requested, paths, skipped);
                    }
                    Phase::Requested(requested) => {
                        let requested = requested.clone();
                        let skipped = !state.ready && state.committed_hooks.close.is_some();
                        let paths = state.cleanup_paths.clone();
                        state.phase = Phase::Finishing;
                        break (requested, paths, skipped);
                    }
                    Phase::Finishing => {
                        state = self.completed.wait(state).expect("shutdown wait poisoned");
                    }
                    Phase::Finished(completion) => return completion.clone(),
                }
            }
        };

        self.reap_wait();
        for path in cleanup_paths {
            let _ = std::fs::remove_file(path);
        }
        let hook = if skipped_before_ready {
            CloseHookOutcome::SkippedBeforeReady
        } else {
            self.run_close_hook(requested.hook.as_deref(), &requested.reason)
        };
        let completion = ShutdownCompletion {
            reason: requested.reason,
            hook,
        };

        let mut state = self.state.lock().expect("shutdown mutex poisoned");
        state.phase = Phase::Finished(completion.clone());
        self.completed.notify_all();
        completion
    }

    fn run_close_hook(&self, command: Option<&str>, reason: &ShutdownReason) -> CloseHookOutcome {
        let Some(command) = command else {
            return CloseHookOutcome::NotConfigured;
        };
        if self.verbose {
            diagnostics::debug(&diagnostics::Record {
                source: Some("close_hook"),
                decision: Some("started"),
                note: Some(format!("reason={} command={command}", reason.label())),
                ..Default::default()
            });
        }
        let context = TaskContext {
            cwd: Some(self.root.clone()),
            ..TaskContext::default()
        };
        let mut child = match self.runner.spawn(
            "close hook",
            &CommandLine::Shell(command.to_owned()),
            &context,
            None,
            Some("close hook".to_owned()),
            false,
        ) {
            Ok(child) => child,
            Err(err) => return CloseHookOutcome::Failed(err),
        };
        let deadline = Instant::now() + self.hook_timeout;
        loop {
            if self.accelerate.load(Ordering::SeqCst) {
                let (signal, grace) = crate::process_owner::shutdown_policy();
                child.shutdown(signal, grace, self.verbose);
                return CloseHookOutcome::Cancelled;
            }
            match child.try_wait() {
                Ok(Some(status)) if status.success() => return CloseHookOutcome::Passed,
                Ok(Some(status)) => return CloseHookOutcome::Failed(status.to_string()),
                Err(err) => return CloseHookOutcome::Failed(err.to_string()),
                Ok(None) if Instant::now() >= deadline => {
                    let (signal, grace) = crate::process_owner::shutdown_policy();
                    child.shutdown(signal, grace, self.verbose);
                    return CloseHookOutcome::TimedOut;
                }
                Ok(None) => std::thread::sleep(Duration::from_millis(10)),
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::cmd::ShutdownOutcome;
    use crate::executor::ChildProcess;
    use std::io;
    #[cfg(unix)]
    use std::os::unix::process::ExitStatusExt;
    use std::process::ExitStatus;
    use std::sync::atomic::{AtomicUsize, Ordering};

    struct FakeReaper(AtomicUsize);
    impl ProcessReaper for FakeReaper {
        fn reap(&self, _verbose: bool) {
            self.0.fetch_add(1, Ordering::SeqCst);
        }
    }

    struct FakeRunner {
        commands: Mutex<Vec<String>>,
        mode: FakeMode,
        shutdowns: Arc<AtomicUsize>,
    }

    #[derive(Clone, Copy)]
    enum FakeMode {
        Pass,
        Fail,
        Hang,
    }

    struct FakeChild {
        mode: FakeMode,
        shutdowns: Arc<AtomicUsize>,
    }

    impl ChildProcess for FakeChild {
        fn try_wait(&mut self) -> io::Result<Option<ExitStatus>> {
            Ok(match self.mode {
                FakeMode::Pass => Some(ExitStatus::from_raw(0)),
                FakeMode::Fail => Some(ExitStatus::from_raw(1 << 8)),
                FakeMode::Hang => None,
            })
        }

        fn shutdown(
            &mut self,
            _signal: nix::sys::signal::Signal,
            _grace: Duration,
            _verbose: bool,
        ) -> ShutdownOutcome {
            self.shutdowns.fetch_add(1, Ordering::SeqCst);
            ShutdownOutcome::AlreadyExited(ExitStatus::from_raw(0))
        }
    }

    impl ProcessRunner for FakeRunner {
        fn spawn(
            &self,
            _task: &str,
            command: &CommandLine,
            _context: &TaskContext,
            _capture: Option<Arc<crate::cmd::CaptureHandle>>,
            _label: Option<String>,
            _quiet: bool,
        ) -> Result<Box<dyn ChildProcess>, String> {
            let CommandLine::Shell(command) = command else {
                return Err("expected shell".to_owned());
            };
            self.commands.lock().unwrap().push(command.clone());
            Ok(Box::new(FakeChild {
                mode: self.mode,
                shutdowns: Arc::clone(&self.shutdowns),
            }))
        }
    }

    fn coordinator(
        hook: &str,
        mode: FakeMode,
        timeout: Duration,
    ) -> (Arc<ShutdownCoordinator>, Arc<FakeRunner>, Arc<FakeReaper>) {
        let runner = Arc::new(FakeRunner {
            commands: Mutex::new(Vec::new()),
            mode,
            shutdowns: Arc::new(AtomicUsize::new(0)),
        });
        let reaper = Arc::new(FakeReaper(AtomicUsize::new(0)));
        let coordinator = Arc::new(ShutdownCoordinator::new(
            std::env::current_dir().unwrap(),
            SessionHooks {
                close: Some(hook.to_owned()),
            },
            false,
            runner.clone(),
            reaper.clone(),
            timeout,
        ));
        coordinator.mark_ready();
        (coordinator, runner, reaper)
    }

    /// TASK-0162 regression proof: the signal thread claims and starts the
    /// shared reap (grace loop + SIGKILL escalation); `finish` must block on
    /// that in-flight reap and only cross the close boundary after it
    /// completed, so `process::exit` can never preempt the escalation.
    #[test]
    fn finish_waits_for_in_flight_reap_completion() {
        use std::sync::mpsc;

        struct BlockingReaper {
            entered: mpsc::Sender<()>,
            release: std::sync::atomic::AtomicBool,
            runs: AtomicUsize,
        }
        impl ProcessReaper for BlockingReaper {
            fn reap(&self, _verbose: bool) {
                self.runs.fetch_add(1, Ordering::SeqCst);
                let _ = self.entered.send(());
                while !self.release.load(std::sync::atomic::Ordering::SeqCst) {
                    std::thread::sleep(Duration::from_millis(5));
                }
            }
        }

        let (entered_tx, entered_rx) = mpsc::channel();
        let reaper = Arc::new(BlockingReaper {
            entered: entered_tx,
            release: std::sync::atomic::AtomicBool::new(false),
            runs: AtomicUsize::new(0),
        });
        let coordinator = Arc::new(ShutdownCoordinator::new(
            std::env::current_dir().unwrap(),
            SessionHooks { close: None },
            false,
            Arc::new(FakeRunner {
                commands: Mutex::new(Vec::new()),
                mode: FakeMode::Pass,
                shutdowns: Arc::new(AtomicUsize::new(0)),
            }),
            reaper.clone(),
            Duration::from_secs(5),
        ));
        coordinator.mark_ready();

        // Signal thread path: request() -> reap_once() starts and blocks in
        // the reaper until released.
        let signal_coordinator = Arc::clone(&coordinator);
        let signal_thread = std::thread::spawn(move || {
            assert!(signal_coordinator.request(ShutdownReason::Signal {
                name: "SIGTERM",
                exit_code: 143,
            }));
        });
        // Deterministic rendezvous: wait until the reaper actually entered.
        entered_rx
            .recv_timeout(Duration::from_secs(5))
            .expect("reaper never started");

        let (finished, completion_rx) = mpsc::channel();
        let finish_coordinator = Arc::clone(&coordinator);
        std::thread::spawn(move || {
            finish_coordinator.finish();
            let _ = finished.send(());
        });
        // finish() must still be parked on the in-flight reap (bounded
        // negative check: a completion message within the window would fail
        // the assertion; the reaper provably cannot finish while unreleased).
        assert!(
            completion_rx
                .recv_timeout(Duration::from_millis(250))
                .is_err(),
            "finish crossed the close boundary before the in-flight reap completed"
        );

        reaper
            .release
            .store(true, std::sync::atomic::Ordering::SeqCst);
        assert!(
            completion_rx.recv_timeout(Duration::from_secs(5)).is_ok(),
            "finish never completed after the reap finished"
        );
        signal_thread.join().unwrap();
        assert_eq!(
            reaper.runs.load(Ordering::SeqCst),
            1,
            "reap must stay exactly-once across request and finish"
        );
    }

    #[test]
    fn first_reason_and_hook_run_exactly_once_for_concurrent_finishers() {
        let (coordinator, runner, reaper) =
            coordinator("echo once", FakeMode::Pass, Duration::from_secs(1));
        assert!(coordinator.request(ShutdownReason::Signal {
            name: "SIGINT",
            exit_code: 130,
        }));
        let first = Arc::clone(&coordinator);
        let second = Arc::clone(&coordinator);
        let a = std::thread::spawn(move || first.finish());
        let b = std::thread::spawn(move || second.finish());
        assert_eq!(a.join().unwrap(), b.join().unwrap());
        assert_eq!(runner.commands.lock().unwrap().as_slice(), ["echo once"]);
        assert_eq!(reaper.0.load(Ordering::SeqCst), 1);
    }

    #[test]
    fn request_snapshots_latest_committed_hook_and_ignores_later_update() {
        let (coordinator, runner, _) =
            coordinator("echo old", FakeMode::Pass, Duration::from_secs(1));
        coordinator.update_hooks(SessionHooks {
            close: Some("echo latest".to_owned()),
        });
        assert!(coordinator.request(ShutdownReason::Normal));
        coordinator.update_hooks(SessionHooks {
            close: Some("echo too-late".to_owned()),
        });
        coordinator.finish();
        assert_eq!(runner.commands.lock().unwrap().as_slice(), ["echo latest"]);
    }

    #[test]
    fn hook_failure_never_replaces_original_exit_code() {
        let (coordinator, _, _) = coordinator("exit 7", FakeMode::Fail, Duration::from_secs(1));
        coordinator.request(ShutdownReason::Signal {
            name: "SIGTERM",
            exit_code: 143,
        });
        let completion = coordinator.finish();
        assert_eq!(completion.reason.exit_code(), 143);
        assert!(matches!(completion.hook, CloseHookOutcome::Failed(_)));
    }

    #[test]
    fn hook_timeout_cancels_and_reaps_child() {
        let (coordinator, runner, _) =
            coordinator("sleep forever", FakeMode::Hang, Duration::from_millis(1));
        coordinator.request(ShutdownReason::Normal);
        let completion = coordinator.finish();
        assert_eq!(completion.hook, CloseHookOutcome::TimedOut);
        assert_eq!(runner.shutdowns.load(Ordering::SeqCst), 1);
        assert_eq!(completion.reason.exit_code(), 0);
    }

    #[test]
    fn configured_hook_is_skipped_before_watcher_readiness() {
        let runner = Arc::new(FakeRunner {
            commands: Mutex::new(Vec::new()),
            mode: FakeMode::Pass,
            shutdowns: Arc::new(AtomicUsize::new(0)),
        });
        let reaper = Arc::new(FakeReaper(AtomicUsize::new(0)));
        let coordinator = ShutdownCoordinator::new(
            std::env::current_dir().unwrap(),
            SessionHooks {
                close: Some("echo no".to_owned()),
            },
            false,
            runner.clone(),
            reaper,
            Duration::from_secs(1),
        );
        coordinator.request(ShutdownReason::Normal);
        assert_eq!(
            coordinator.finish().hook,
            CloseHookOutcome::SkippedBeforeReady
        );
        assert!(runner.commands.lock().unwrap().is_empty());
    }
}
