//! Interactive recovery approval adapters.
//!
//! The executor consumes an injected approval port; this module is the
//! foreground CLI's TTY adapter. Headless callers get a bounded default-deny
//! response rather than a read that can block forever.

use crate::executor::{ApprovalDecision, CancellationToken, RecoveryApproval, RecoveryRequest};
use std::io::{self, IsTerminal, Write};
#[cfg(unix)]
use std::os::unix::io::AsRawFd;
use std::time::{Duration, Instant};

#[derive(Default)]
pub struct TtyRecoveryApproval;

#[cfg(unix)]
fn flush_pending_input(fd: std::os::unix::io::RawFd) {
    // Best effort: the TTY remains the transport boundary, and an input
    // flush is safer than allowing stale canonical bytes to cross a
    // generation boundary. The next prompt starts with a clean line.
    unsafe {
        let _ = nix::libc::tcflush(fd, nix::libc::TCIFLUSH);
    }
}

impl RecoveryApproval for TtyRecoveryApproval {
    fn approve(
        &self,
        requests: &[RecoveryRequest],
        cancellation: &CancellationToken,
        timeout: Duration,
    ) -> ApprovalDecision {
        let stdin = io::stdin();
        let stdout = io::stdout();
        if !stdin.is_terminal() || !stdout.is_terminal() {
            return ApprovalDecision::NoTty;
        }

        let mut output = stdout.lock();
        let generation = requests
            .first()
            .map(|request| request.generation)
            .unwrap_or_default();
        let jobs = requests
            .iter()
            .map(|request| request.job.as_str())
            .collect::<Vec<_>>()
            .join(", ");
        let _ = writeln!(output, "Generation {generation} failed in jobs: {jobs}");
        let _ = writeln!(output, "Proposed recoveries (run once, in this order):");
        for request in requests {
            let _ = writeln!(
                output,
                "  [{}] (position={})",
                request.job, request.job_position
            );

            for (index, command) in request.commands.iter().enumerate() {
                let _ = writeln!(output, "    {}. {}", index + 1, command);
            }
        }
        let _ = writeln!(output, "These commands may mutate the workspace.");
        let _ = write!(
            output,
            "Run these recoveries and verify the failed jobs? [y/N] "
        );
        let _ = output.flush();
        drop(output);

        let mut answer = String::new();
        #[cfg(unix)]
        {
            let fd = stdin.as_raw_fd();
            // Input typed for a generation that was cancelled or superseded
            // must never become an approval for its successor. Discard any
            // bytes left in the terminal's canonical input queue before
            // presenting this generation's prompt.
            flush_pending_input(fd);
            let started = Instant::now();
            let deadline = started + timeout;
            let mut poll = nix::libc::pollfd {
                fd,
                events: nix::libc::POLLIN,
                revents: 0,
            };
            loop {
                if cancellation.is_cancelled() {
                    flush_pending_input(fd);
                    return ApprovalDecision::Cancelled;
                }
                let remaining = deadline.saturating_duration_since(Instant::now());
                if remaining.is_zero() {
                    return ApprovalDecision::TimedOut;
                }
                poll.revents = 0;
                // Keep cancellation responsive even though libc::poll has no
                // cancellation-fd integration here. The executor waits for a
                // cooperative TTY adapter before promoting a successor.
                let wait_ms = remaining
                    .min(Duration::from_millis(50))
                    .as_millis()
                    .max(1)
                    .min(i32::MAX as u128) as i32;
                let ready = unsafe { nix::libc::poll(&mut poll, 1, wait_ms) };
                if ready < 0 {
                    if std::io::Error::last_os_error().kind() == std::io::ErrorKind::Interrupted {
                        continue;
                    }
                    return ApprovalDecision::Eof;
                }
                if ready == 0 {
                    if Instant::now() >= deadline {
                        return ApprovalDecision::TimedOut;
                    }
                    continue;
                }
                let mut buffer = [0u8; 256];
                let read = unsafe { nix::libc::read(fd, buffer.as_mut_ptr().cast(), buffer.len()) };
                if read == 0 {
                    return ApprovalDecision::Eof;
                }
                if read < 0 {
                    let error = std::io::Error::last_os_error();
                    if error.kind() == std::io::ErrorKind::Interrupted
                        || error.kind() == std::io::ErrorKind::WouldBlock
                    {
                        continue;
                    }
                    return ApprovalDecision::Eof;
                }
                answer.push_str(&String::from_utf8_lossy(&buffer[..read as usize]));
                if answer.contains('\n') {
                    break;
                }
            }
        }
        #[cfg(not(unix))]
        {
            if cancellation.is_cancelled() {
                return ApprovalDecision::Cancelled;
            }
            if stdin.read_line(&mut answer).is_err() {
                return ApprovalDecision::Eof;
            }
        }
        match answer.trim().to_ascii_lowercase().as_str() {
            "y" | "yes" => ApprovalDecision::Approved,
            "" => ApprovalDecision::Eof,
            "n" | "no" => ApprovalDecision::Declined,
            _ => ApprovalDecision::Invalid,
        }
    }
}
