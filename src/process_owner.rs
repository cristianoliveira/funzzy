//! Child process-group ownership (TASK-0030).
//!
//! Each spawned task leads its own process group (see `cmd::spawn_configured`).
//! This module tracks those groups so that every shutdown path — Ctrl-C,
//! restart replacement, config reload, worker drop — reaches the whole task
//! tree (shell + descendants) through one ownership path instead of signaling
//! only the direct child PID and orphaning grandchildren.
//!
//! The registry is process-global because SIGINT delivery happens on a thread
//! that does not own any `Worker`/`Run` handle.
//!
//! This implementation is explicitly Unix-only: it uses process groups and
//! `nix` signals. Unsupported platforms fail at build time rather than
//! silently falling back to direct-PID cancellation that could orphan children.

use nix::sys::signal::{self, Signal};
use nix::unistd::Pid;
use std::sync::Mutex;
use std::time::{Duration, Instant};

use crate::stdout;

/// Registered owned process-group IDs (each equals the leader's PID, since
/// tasks call `setpgid(0, 0)` before exec).
static OWNED_GROUPS: Mutex<Vec<i32>> = Mutex::new(Vec::new());

/// Records a process group as owned by this funzzy process. Idempotent.
pub fn register(pgid: i32) {
    let mut groups = OWNED_GROUPS.lock().expect("owned groups mutex poisoned");
    if !groups.contains(&pgid) {
        groups.push(pgid);
    }
}

/// Forgets an owned process group (e.g., after it exited and was reaped).
pub fn unregister(pgid: i32) {
    let mut groups = OWNED_GROUPS.lock().expect("owned groups mutex poisoned");
    groups.retain(|g| *g != pgid);
}

/// True if any process still exists in the group (`kill(-pgid, 0)` probe).
fn group_alive(pgid: i32) -> bool {
    match signal::kill(Pid::from_raw(-pgid), None) {
        Ok(()) => true,
        Err(nix::errno::Errno::ESRCH) => false,
        Err(_) => true,
    }
}

/// Shuts every owned process group down: sends `signal` to the whole group,
/// waits up to `grace`, then force-kills (`SIGKILL`) whatever remains.
///
/// This is the Ctrl-C / process-shutdown path. Normal per-run cancellation
/// (`executor::Run::cancel`) reaps via the owned `LoggedChild`; this path is
/// used when the whole process is stopping and just needs to reach every
/// descendant group.
pub fn shutdown_all(signal: Signal, grace: Duration, verbose: bool) -> ShutdownTally {
    let groups: Vec<i32> = OWNED_GROUPS
        .lock()
        .expect("owned groups mutex poisoned")
        .iter()
        .copied()
        .collect();

    let mut tally = ShutdownTally {
        signaled: 0,
        force_killed: 0,
    };
    if groups.is_empty() {
        return tally;
    }

    // 1. Initial signal to every owned group.
    for pgid in &groups {
        stdout::verbose(
            &format!(
                "---- signalling process group -{} with {:?} ----",
                pgid, signal
            ),
            verbose,
        );
        let _ = signal::kill(Pid::from_raw(-*pgid), signal);
    }

    // 2. Wait up to grace for groups to vanish.
    let deadline = Instant::now() + grace;
    let mut pending = groups.clone();
    while Instant::now() < deadline && !pending.is_empty() {
        std::thread::sleep(Duration::from_millis(20));
        pending.retain(|pgid| group_alive(*pgid));
    }

    tally.signaled = (groups.len() - pending.len()) as u32;

    // 3. Escalate: SIGKILL any group still alive.
    for pgid in &pending {
        stdout::verbose(
            &format!(
                "---- grace of {:?} elapsed; force-killing process group -{} ----",
                grace, pgid
            ),
            verbose,
        );
        let _ = signal::kill(Pid::from_raw(-*pgid), Signal::SIGKILL);
    }
    tally.force_killed = pending.len() as u32;

    tally
}

/// Summary of a `shutdown_all` operation.
#[derive(Debug, Default, Clone, Copy, PartialEq, Eq)]
pub struct ShutdownTally {
    /// Groups that terminated after the initial signal, within grace.
    pub signaled: u32,
    /// Groups that had to be force-killed after the grace period.
    pub force_killed: u32,
}

/// The cancel signal and grace duration used by shutdown paths. Safe defaults
/// (`SIGTERM`, 5s), overridable for tests via `FUNZZY_CANCEL_GRACE_MS`.
pub fn shutdown_policy() -> (Signal, Duration) {
    let signal_value = std::env::var("FUNZZY_CANCEL_SIGNAL").ok();
    let signal = shutdown_signal(signal_value.as_deref());
    let grace_ms = std::env::var("FUNZZY_CANCEL_GRACE_MS")
        .ok()
        .and_then(|v| v.parse::<u64>().ok())
        .unwrap_or(5_000);
    (signal, Duration::from_millis(grace_ms))
}

/// Parses the configured graceful-cancel signal. Unsupported values fall
/// back deterministically to SIGTERM rather than silently disabling cleanup.
fn shutdown_signal(value: Option<&str>) -> Signal {
    match value.map(str::trim).map(str::to_ascii_uppercase).as_deref() {
        Some("INT") | Some("SIGINT") => Signal::SIGINT,
        Some("HUP") | Some("SIGHUP") => Signal::SIGHUP,
        Some("QUIT") | Some("SIGQUIT") => Signal::SIGQUIT,
        Some("TERM") | Some("SIGTERM") | None => Signal::SIGTERM,
        Some(_) => Signal::SIGTERM,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    // A PID no real process occupies; unique to this test module so concurrent
    // cmd/workers tests (which register real PIDs) cannot collide with it.
    const OWNED_BY_THIS_TEST: i32 = 999_991;

    #[test]
    fn register_then_unregister_forgets_the_group() {
        register(OWNED_BY_THIS_TEST);
        let still_present = OWNED_GROUPS
            .lock()
            .expect("mutex")
            .contains(&OWNED_BY_THIS_TEST);
        assert!(still_present, "register must track the group");

        unregister(OWNED_BY_THIS_TEST);
        let still_there = OWNED_GROUPS
            .lock()
            .expect("mutex")
            .contains(&OWNED_BY_THIS_TEST);
        assert!(!still_there, "unregister must forget the group");
    }

    #[test]
    fn register_is_idempotent() {
        register(OWNED_BY_THIS_TEST);
        register(OWNED_BY_THIS_TEST);
        let count = OWNED_GROUPS
            .lock()
            .expect("mutex")
            .iter()
            .filter(|&&g| g == OWNED_BY_THIS_TEST)
            .count();
        assert_eq!(count, 1, "registering twice must not duplicate");
        unregister(OWNED_BY_THIS_TEST);
    }

    #[test]
    fn shutdown_policy_defaults_to_sigterm_and_five_seconds() {
        // Ensure we don't accidentally inherit a test env override.
        std::env::remove_var("FUNZZY_CANCEL_SIGNAL");
        std::env::remove_var("FUNZZY_CANCEL_GRACE_MS");
        let (signal, grace) = shutdown_policy();
        assert_eq!(signal, Signal::SIGTERM);
        assert_eq!(grace, Duration::from_millis(5_000));
    }

    #[test]
    fn shutdown_signal_accepts_supported_values_and_defaults_invalid_values() {
        assert_eq!(shutdown_signal(Some("INT")), Signal::SIGINT);
        assert_eq!(shutdown_signal(Some("sighup")), Signal::SIGHUP);
        assert_eq!(shutdown_signal(Some("QUIT")), Signal::SIGQUIT);
        assert_eq!(shutdown_signal(Some("TERM")), Signal::SIGTERM);
        assert_eq!(shutdown_signal(Some("not-a-signal")), Signal::SIGTERM);
        assert_eq!(shutdown_signal(None), Signal::SIGTERM);
    }
}
