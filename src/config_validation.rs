//! Pure validation of neutral configuration inputs.
//!
//! YAML decoding and error presentation stay in `config`; this module owns
//! cross-field policy that can be checked without filesystem or runtime
//! dependencies. Keep the input small until another real validation boundary
//! needs to move here.

/// The neutral facts needed to validate a manual job's trigger surface.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) struct ManualTriggerInput<'a> {
    pub(crate) manual: bool,
    pub(crate) change_patterns: &'a [String],
    pub(crate) ignore_patterns: &'a [String],
}

/// Cross-field violation in a manual job's filesystem-trigger surface.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum ManualTriggerViolation {
    Change,
    Ignore,
}

/// Validates manual-trigger combinations without decoding YAML or observing
/// the filesystem. The order is contract-significant: `change` wins when both
/// conflicting fields are present, matching the historical parser behavior.
pub(crate) fn validate_manual_trigger(
    input: ManualTriggerInput<'_>,
) -> Result<(), ManualTriggerViolation> {
    if !input.manual {
        return Ok(());
    }
    if !input.change_patterns.is_empty() {
        return Err(ManualTriggerViolation::Change);
    }
    if !input.ignore_patterns.is_empty() {
        return Err(ManualTriggerViolation::Ignore);
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn input<'a>(
        manual: bool,
        change_patterns: &'a [String],
        ignore_patterns: &'a [String],
    ) -> ManualTriggerInput<'a> {
        ManualTriggerInput {
            manual,
            change_patterns,
            ignore_patterns,
        }
    }

    #[test]
    fn non_manual_jobs_allow_trigger_patterns() {
        let change = vec![String::from("src/**")];
        let ignore = vec![String::from("**/*.log")];

        assert_eq!(
            validate_manual_trigger(input(false, &change, &ignore)),
            Ok(())
        );
    }

    #[test]
    fn manual_jobs_allow_an_empty_trigger_surface() {
        assert_eq!(validate_manual_trigger(input(true, &[], &[])), Ok(()));
    }

    #[test]
    fn manual_change_is_rejected_before_ignore() {
        let change = vec![String::from("src/**")];
        let ignore = vec![String::from("**/*.log")];

        assert_eq!(
            validate_manual_trigger(input(true, &change, &ignore)),
            Err(ManualTriggerViolation::Change)
        );
    }

    #[test]
    fn manual_ignore_is_rejected_when_change_is_absent() {
        let ignore = vec![String::from("**/*.log")];

        assert_eq!(
            validate_manual_trigger(input(true, &[], &ignore)),
            Err(ManualTriggerViolation::Ignore)
        );
    }
}
