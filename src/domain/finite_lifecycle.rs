//! Pure finite-job lifecycle decisions.
//!
//! Runtime adapters translate process observations into these domain values.
//! This module deliberately has no process, clock, output, or cancellation
//! handle dependencies.

/// Result of observing one finite-job command.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum ProcessResult {
    /// The task has work ready to spawn.
    NotStarted,
    /// The command is still running.
    Running,
    /// The command exited successfully.
    Succeeded,
    /// The command exited unsuccessfully or could not be observed.
    Failed,
}

/// Inputs that may compete at one finite-job observation boundary.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) struct Observation {
    /// A cancellation command has been accepted for this generation.
    pub(crate) cancellation_requested: bool,
    /// The job-wide deadline has elapsed.
    pub(crate) deadline_elapsed: bool,
    /// The process adapter's semantic observation.
    pub(crate) process: ProcessResult,
}

/// Policy for a failed command after the failure has been recorded.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum FailureAction {
    /// Keep running later work and do not offer recovery.
    Continue,
    /// Stop sibling/later work immediately and do not offer recovery.
    FailFast,
    /// Defer the failed job to the recovery pass.
    Recover,
    /// Stop sibling/later work, then resolve the deferred recovery pass.
    FailFastAndRecover,
}

/// Pure decision at one finite-job observation boundary.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum Decision {
    Cancelled,
    TimedOut,
    Start,
    Continue,
    Passed,
    Failed(FailureAction),
}

/// Resolves competing finite-job inputs with deterministic precedence.
///
/// Cancellation outranks timeout, and timeout outranks every process result.
/// A running process continues; a successful process passes; an unsuccessful
/// process follows the fail-fast/recovery policy. The caller remains
/// responsible for recording messages, terminating children, and publishing
/// outcomes.
pub(crate) fn resolve(
    observation: Observation,
    fail_fast: bool,
    recovery_eligible: bool,
) -> Decision {
    if observation.cancellation_requested {
        return Decision::Cancelled;
    }
    if observation.deadline_elapsed {
        return Decision::TimedOut;
    }
    match observation.process {
        ProcessResult::NotStarted => Decision::Start,
        ProcessResult::Running => Decision::Continue,
        ProcessResult::Succeeded => Decision::Passed,
        ProcessResult::Failed => Decision::Failed(match (fail_fast, recovery_eligible) {
            (false, false) => FailureAction::Continue,
            (true, false) => FailureAction::FailFast,
            (false, true) => FailureAction::Recover,
            (true, true) => FailureAction::FailFastAndRecover,
        }),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn observation(process: ProcessResult) -> Observation {
        Observation {
            cancellation_requested: false,
            deadline_elapsed: false,
            process,
        }
    }

    #[test]
    fn each_process_result_has_a_decision() {
        assert_eq!(
            resolve(observation(ProcessResult::NotStarted), false, false),
            Decision::Start
        );
        assert_eq!(
            resolve(observation(ProcessResult::Running), false, false),
            Decision::Continue
        );
        assert_eq!(
            resolve(observation(ProcessResult::Succeeded), false, false),
            Decision::Passed
        );
        assert_eq!(
            resolve(observation(ProcessResult::Failed), false, false),
            Decision::Failed(FailureAction::Continue)
        );
    }

    #[test]
    fn cancellation_wins_over_timeout_and_process_result() {
        for process in [
            ProcessResult::NotStarted,
            ProcessResult::Running,
            ProcessResult::Succeeded,
            ProcessResult::Failed,
        ] {
            assert_eq!(
                resolve(
                    Observation {
                        cancellation_requested: true,
                        deadline_elapsed: true,
                        process,
                    },
                    true,
                    true,
                ),
                Decision::Cancelled
            );
        }
    }

    #[test]
    fn timeout_wins_over_process_result() {
        for process in [
            ProcessResult::NotStarted,
            ProcessResult::Running,
            ProcessResult::Succeeded,
            ProcessResult::Failed,
        ] {
            assert_eq!(
                resolve(
                    Observation {
                        cancellation_requested: false,
                        deadline_elapsed: true,
                        process,
                    },
                    true,
                    true,
                ),
                Decision::TimedOut
            );
        }
    }

    #[test]
    fn failure_action_preserves_fail_fast_and_recovery_eligibility() {
        assert_eq!(
            resolve(observation(ProcessResult::Failed), false, false),
            Decision::Failed(FailureAction::Continue)
        );
        assert_eq!(
            resolve(observation(ProcessResult::Failed), true, false),
            Decision::Failed(FailureAction::FailFast)
        );
        assert_eq!(
            resolve(observation(ProcessResult::Failed), false, true),
            Decision::Failed(FailureAction::Recover)
        );
        assert_eq!(
            resolve(observation(ProcessResult::Failed), true, true),
            Decision::Failed(FailureAction::FailFastAndRecover)
        );
    }

    #[test]
    fn an_empty_running_observation_continues() {
        assert_eq!(
            resolve(observation(ProcessResult::Running), false, false),
            Decision::Continue
        );
    }
}
