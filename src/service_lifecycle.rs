//! Deterministic service-readiness arbitration primitives.
//!
//! The worker owns process handles; this module owns only the precedence rules
//! used when commands and child observations arrive in one worker cycle.

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn cancellation_wins_over_probe_success_in_the_same_cycle() {
        assert_eq!(
            ReadinessArbiter::resolve(
                &[ArbitrationCommand::Cancel],
                &[ChildObservation::ReadinessExit { success: true }],
            ),
            ArbitrationDecision::Cancelled
        );
    }

    #[test]
    fn service_exit_wins_over_timeout_and_probe_exit() {
        assert_eq!(
            ReadinessArbiter::resolve(
                &[],
                &[
                    ChildObservation::ReadinessExit { success: true },
                    ChildObservation::ReadinessTimeout,
                    ChildObservation::ServiceExit,
                ],
            ),
            ArbitrationDecision::ServiceExited
        );
    }

    #[test]
    fn timeout_wins_over_probe_success_at_the_deadline() {
        assert_eq!(
            ReadinessArbiter::resolve(
                &[],
                &[
                    ChildObservation::ReadinessExit { success: true },
                    ChildObservation::ReadinessTimeout,
                ],
            ),
            ArbitrationDecision::ReadinessTimedOut
        );
    }

    #[test]
    fn command_precedence_is_stable_independent_of_input_order() {
        let commands = [
            ArbitrationCommand::ReloadReplacement,
            ArbitrationCommand::Supersede,
            ArbitrationCommand::Shutdown,
            ArbitrationCommand::Cancel,
        ];
        assert_eq!(
            ReadinessArbiter::resolve(&commands, &[]),
            ArbitrationDecision::Shutdown
        );
        assert_eq!(
            ReadinessArbiter::resolve(
                &[ArbitrationCommand::ReloadReplacement, ArbitrationCommand::Cancel],
                &[],
            ),
            ArbitrationDecision::Cancelled
        );
    }

    #[test]
    fn failed_probe_is_retryable_and_not_a_terminal_generation_failure() {
        assert_eq!(
            ReadinessArbiter::resolve(
                &[],
                &[ChildObservation::ReadinessExit { success: false }],
            ),
            ArbitrationDecision::RetryReadiness
        );
    }
}
