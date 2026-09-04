//! Deterministic service-readiness arbitration primitives.
//!
//! The worker owns process handles; this module owns only the precedence rules
//! used when commands and child observations arrive in one worker cycle.

/// Commands accepted by the worker before an observation-cycle marker.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ArbitrationCommand {
    Shutdown,
    Cancel,
    Supersede,
    ReloadReplacement,
}

/// Child facts observed during one deterministic worker cycle.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ChildObservation {
    ServiceExit,
    ReadinessTimeout,
    ReadinessExit { success: bool },
}

/// The single fact allowed to affect a starting service in one cycle.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ArbitrationDecision {
    Shutdown,
    Cancelled,
    Superseded,
    ReloadReplacement,
    ServiceExited,
    ReadinessTimedOut,
    ReadinessPassed,
    RetryReadiness,
    Noop,
}

/// Stateless precedence resolver. Commands outrank child facts; within each
/// class the contract's explicit precedence order is used, not arrival order.
pub struct ReadinessArbiter;

impl ReadinessArbiter {
    pub fn new() -> Self {
        Self
    }

    pub fn resolve(
        commands: &[ArbitrationCommand],
        observations: &[ChildObservation],
    ) -> ArbitrationDecision {
        for (command, decision) in [
            (ArbitrationCommand::Shutdown, ArbitrationDecision::Shutdown),
            (ArbitrationCommand::Cancel, ArbitrationDecision::Cancelled),
            (
                ArbitrationCommand::Supersede,
                ArbitrationDecision::Superseded,
            ),
            (
                ArbitrationCommand::ReloadReplacement,
                ArbitrationDecision::ReloadReplacement,
            ),
        ] {
            if commands.contains(&command) {
                return decision;
            }
        }
        if observations.contains(&ChildObservation::ServiceExit) {
            return ArbitrationDecision::ServiceExited;
        }
        if observations.contains(&ChildObservation::ReadinessTimeout) {
            return ArbitrationDecision::ReadinessTimedOut;
        }
        if observations.iter().any(|observation| {
            matches!(
                observation,
                ChildObservation::ReadinessExit { success: true }
            )
        }) {
            return ArbitrationDecision::ReadinessPassed;
        }
        if observations.iter().any(|observation| {
            matches!(
                observation,
                ChildObservation::ReadinessExit { success: false }
            )
        }) {
            return ArbitrationDecision::RetryReadiness;
        }
        ArbitrationDecision::Noop
    }
}

impl Default for ReadinessArbiter {
    fn default() -> Self {
        Self::new()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn transition_matrix_covers_each_individual_lifecycle_outcome() {
        let cases = [
            (
                "shutdown",
                vec![ArbitrationCommand::Shutdown],
                vec![],
                ArbitrationDecision::Shutdown,
            ),
            (
                "cancel",
                vec![ArbitrationCommand::Cancel],
                vec![],
                ArbitrationDecision::Cancelled,
            ),
            (
                "supersede",
                vec![ArbitrationCommand::Supersede],
                vec![],
                ArbitrationDecision::Superseded,
            ),
            (
                "reload replacement",
                vec![ArbitrationCommand::ReloadReplacement],
                vec![],
                ArbitrationDecision::ReloadReplacement,
            ),
            (
                "service exit",
                vec![],
                vec![ChildObservation::ServiceExit],
                ArbitrationDecision::ServiceExited,
            ),
            (
                "readiness timeout",
                vec![],
                vec![ChildObservation::ReadinessTimeout],
                ArbitrationDecision::ReadinessTimedOut,
            ),
            (
                "readiness passed",
                vec![],
                vec![ChildObservation::ReadinessExit { success: true }],
                ArbitrationDecision::ReadinessPassed,
            ),
            (
                "readiness retry",
                vec![],
                vec![ChildObservation::ReadinessExit { success: false }],
                ArbitrationDecision::RetryReadiness,
            ),
            ("no-op", vec![], vec![], ArbitrationDecision::Noop),
        ];

        for (name, commands, observations, expected) in cases {
            assert_eq!(
                ReadinessArbiter::resolve(&commands, &observations),
                expected,
                "transition matrix case {name}"
            );
        }
    }

    #[test]
    fn command_precedence_beats_every_observation() {
        let observations = vec![
            ChildObservation::ReadinessExit { success: false },
            ChildObservation::ReadinessExit { success: true },
            ChildObservation::ReadinessTimeout,
            ChildObservation::ServiceExit,
        ];

        for (command, expected) in [
            (ArbitrationCommand::Shutdown, ArbitrationDecision::Shutdown),
            (ArbitrationCommand::Cancel, ArbitrationDecision::Cancelled),
            (
                ArbitrationCommand::Supersede,
                ArbitrationDecision::Superseded,
            ),
            (
                ArbitrationCommand::ReloadReplacement,
                ArbitrationDecision::ReloadReplacement,
            ),
        ] {
            assert_eq!(
                ReadinessArbiter::resolve(&[command], &observations),
                expected,
                "command must beat competing observations: {command:?}"
            );
        }
    }

    #[test]
    fn competing_observations_follow_fixed_precedence_independent_of_order() {
        let observations = [
            ChildObservation::ReadinessExit { success: false },
            ChildObservation::ReadinessExit { success: true },
            ChildObservation::ReadinessTimeout,
            ChildObservation::ServiceExit,
        ];
        let reverse = observations.into_iter().rev().collect::<Vec<_>>();

        assert_eq!(
            ReadinessArbiter::resolve(&[], &observations),
            ArbitrationDecision::ServiceExited
        );
        assert_eq!(
            ReadinessArbiter::resolve(&[], &reverse),
            ArbitrationDecision::ServiceExited
        );

        let without_exit = [
            ChildObservation::ReadinessExit { success: false },
            ChildObservation::ReadinessExit { success: true },
            ChildObservation::ReadinessTimeout,
        ];
        assert_eq!(
            ReadinessArbiter::resolve(&[], &without_exit),
            ArbitrationDecision::ReadinessTimedOut
        );
    }

    #[test]
    fn empty_inputs_are_a_noop() {
        assert_eq!(
            ReadinessArbiter::resolve(&[], &[]),
            ArbitrationDecision::Noop
        );
    }

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
                &[
                    ArbitrationCommand::ReloadReplacement,
                    ArbitrationCommand::Cancel
                ],
                &[],
            ),
            ArbitrationDecision::Cancelled
        );
    }

    #[test]
    fn failed_probe_is_retryable_and_not_a_terminal_generation_failure() {
        assert_eq!(
            ReadinessArbiter::resolve(&[], &[ChildObservation::ReadinessExit { success: false }],),
            ArbitrationDecision::RetryReadiness
        );
    }
}
