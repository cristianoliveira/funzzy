//! Interactive recovery approval adapters.
//!
//! The executor consumes an injected approval port; this module is the
//! foreground CLI's TTY adapter. Headless callers get a bounded default-deny
//! response rather than a read that can block forever.

use crate::executor::{ApprovalDecision, RecoveryApproval, RecoveryRequest};
use std::io::{self, IsTerminal, Write};

#[derive(Default)]
pub struct TtyRecoveryApproval;

impl RecoveryApproval for TtyRecoveryApproval {
    fn approve(&self, requests: &[RecoveryRequest]) -> ApprovalDecision {
        let stdin = io::stdin();
        let stdout = io::stdout();
        if !stdin.is_terminal() || !stdout.is_terminal() {
            return ApprovalDecision::Declined;
        }

        let mut output = stdout.lock();
        let _ = writeln!(
            output,
            "Recovery approval required for the failed generation:"
        );
        for request in requests {
            let _ = writeln!(
                output,
                "  generation={} job={} (position={})",
                request.generation, request.job, request.job_position
            );
            for (index, command) in request.commands.iter().enumerate() {
                let _ = writeln!(output, "    {}. {}", index + 1, command);
            }
        }
        let _ = write!(output, "Run these recoveries once and verify? [y/N] ");
        let _ = output.flush();
        drop(output);

        let mut answer = String::new();
        if stdin.read_line(&mut answer).is_err() {
            return ApprovalDecision::Declined;
        }
        match answer.trim().to_ascii_lowercase().as_str() {
            "y" | "yes" => ApprovalDecision::Approved,
            _ => ApprovalDecision::Declined,
        }
    }
}
