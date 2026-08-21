//! Interactive recovery approval adapters.
//!
//! The executor consumes an injected approval port; this module is the
//! foreground CLI's TTY adapter. Headless callers get a bounded default-deny
//! response rather than a read that can block forever.

use crate::executor::{ApprovalDecision, CancellationToken, RecoveryApproval, RecoveryRequest};
use std::io::{self, IsTerminal, Write};
#[cfg(unix)]
use std::os::unix::io::AsRawFd;

#[derive(Default)]
pub struct TtyRecoveryApproval;

impl RecoveryApproval for TtyRecoveryApproval {
    fn approve(
        &self,
        requests: &[RecoveryRequest],
        cancellation: &CancellationToken,
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
            let mut poll = nix::libc::pollfd {
                fd: stdin.as_raw_fd(),
                events: nix::libc::POLLIN,
                revents: 0,
            };
            loop {
                if cancellation.is_cancelled() {
                    return ApprovalDecision::Cancelled;
                }
                let ready = unsafe { nix::libc::poll(&mut poll, 1, 50) };
                if ready < 0 {
                    if std::io::Error::last_os_error().kind() == std::io::ErrorKind::Interrupted {
                        continue;
                    }
                    return ApprovalDecision::Eof;
                }
                if ready == 0 {
                    continue;
                }
                break;
            }
        }
        #[cfg(not(unix))]
        if cancellation.is_cancelled() {
            return ApprovalDecision::Cancelled;
        }
        if stdin.read_line(&mut answer).is_err() {
            return ApprovalDecision::Eof;
        }
        match answer.trim().to_ascii_lowercase().as_str() {
            "y" | "yes" => ApprovalDecision::Approved,
            "" => ApprovalDecision::Eof,
            "n" | "no" => ApprovalDecision::Declined,
            _ => ApprovalDecision::Invalid,
        }
    }
}
