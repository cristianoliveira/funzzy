//! Pure validation of neutral configuration inputs.
//!
//! YAML decoding and error presentation stay in `config`; this module owns
//! cross-field policy that can be checked without filesystem or runtime
//! dependencies. Keep the input small until another real validation boundary
//! needs to move here.

use crate::rules::{OutputPolicy, Readiness, Rules, TriggerMode};
use std::collections::BTreeMap;
use std::time::Duration;

/// Neutral, already-decoded values needed to construct one domain rule.
///
/// YAML shape and parser errors stay at the edge; this input contains only
/// domain values and explicit parser decisions such as `timeout_declared`.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct RuleInput {
    pub(crate) name: String,
    pub(crate) commands: Vec<String>,
    pub(crate) watch_patterns: Vec<String>,
    pub(crate) ignore_patterns: Vec<String>,
    pub(crate) run_on_init: bool,
    pub(crate) parallel: Option<String>,
    pub(crate) cwd: Option<String>,
    pub(crate) environment: BTreeMap<String, String>,
    pub(crate) inherited_patterns: Vec<String>,
    pub(crate) output: OutputPolicy,
    pub(crate) service: bool,
    pub(crate) trigger: Option<TriggerMode>,
    pub(crate) timeout: Option<Duration>,
    pub(crate) timeout_declared: bool,
    pub(crate) recovery: Option<Vec<String>>,
    pub(crate) readiness: Option<Readiness>,
}

/// A pure domain-validation failure. The config adapter maps this to its
/// public `FzzError` while retaining the established text and hint.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct ValidationError {
    pub(crate) message: String,
    pub(crate) hint: Option<String>,
}

fn validation_error(message: String, hint: Option<String>) -> ValidationError {
    ValidationError { message, hint }
}

pub(crate) fn validate_service_recovery(
    name: &str,
    service: bool,
    recovery_present: bool,
) -> Result<(), ValidationError> {
    if service && recovery_present {
        return Err(validation_error(
            format!(
                "Job '{}' cannot declare recovery when service is true",
                name
            ),
            Some(
                "A service has no finite verification boundary; remove `recovery` or `service`."
                    .to_owned(),
            ),
        ));
    }
    Ok(())
}

pub(crate) fn validate_manual_lifecycle(
    name: &str,
    trigger: Option<TriggerMode>,
    run_on_init: bool,
    service: bool,
) -> Result<(), ValidationError> {
    if matches!(trigger, Some(TriggerMode::Manual)) && run_on_init {
        return Err(validation_error(
            format!(
                "Job '{}' cannot declare both 'trigger: manual' and 'run_on_init'",
                name
            ),
            Some(
                "Manual jobs never run at watcher initialization; remove 'run_on_init' or 'trigger: manual'."
                    .to_owned(),
            ),
        ));
    }
    if matches!(trigger, Some(TriggerMode::Manual)) && service {
        return Err(validation_error(
            format!(
                "Job '{}' cannot declare both 'trigger: manual' and 'service: true'",
                name
            ),
            Some(
                "Services start on init and restart on change; that contradicts 'trigger: manual'. Remove one."
                    .to_owned(),
            ),
        ));
    }
    Ok(())
}

pub(crate) fn validate_service_timeout(
    name: &str,
    timeout_declared: bool,
    timeout: Option<Duration>,
    service: bool,
) -> Result<(), ValidationError> {
    if timeout_declared && timeout.is_some() && service {
        return Err(validation_error(
            format!(
                "Job '{}' cannot declare both 'timeout' and 'service: true'",
                name
            ),
            Some("A service is intentionally unbounded; remove 'timeout' or 'service'.".to_owned()),
        ));
    }
    Ok(())
}

/// Validates cross-field rule constraints and constructs the domain value.
/// This function performs no YAML decoding or filesystem/runtime work.
pub(crate) fn build_rule(input: RuleInput) -> Result<Rules, ValidationError> {
    let RuleInput {
        name,
        commands,
        watch_patterns,
        ignore_patterns,
        run_on_init,
        parallel,
        cwd,
        environment,
        inherited_patterns,
        output,
        service,
        trigger,
        timeout,
        timeout_declared,
        recovery,
        readiness,
    } = input;

    validate_service_recovery(&name, service, recovery.is_some())?;
    validate_manual_lifecycle(&name, trigger, run_on_init, service)?;
    validate_service_timeout(&name, timeout_declared, timeout, service)?;

    let rule = Rules::new(name, commands, watch_patterns, ignore_patterns, run_on_init)
        .with_execution_context(cwd, environment)
        .with_inherited_patterns(inherited_patterns)
        .with_output(output)
        .with_service(service)
        .with_trigger(trigger)
        .with_timeout(if service { None } else { timeout })
        .with_readiness(readiness);
    let rule = match recovery {
        Some(commands) => rule.with_recovery(commands),
        None => rule,
    };
    Ok(match parallel {
        Some(group) => rule.with_parallel(group),
        None => rule,
    })
}

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

    fn rule_input() -> RuleInput {
        RuleInput {
            name: String::from("build"),
            commands: vec![String::from("cargo build")],
            watch_patterns: vec![String::from("src/**")],
            ignore_patterns: vec![],
            run_on_init: false,
            parallel: None,
            cwd: None,
            environment: BTreeMap::new(),
            inherited_patterns: vec![],
            output: OutputPolicy::Inherit,
            service: false,
            trigger: None,
            timeout: None,
            timeout_declared: false,
            recovery: None,
            readiness: None,
        }
    }

    #[test]
    fn builds_a_domain_rule_from_neutral_input() {
        let rule = build_rule(rule_input()).expect("neutral input is valid");

        assert_eq!(rule.name, "build");
        assert_eq!(rule.commands(), vec!["cargo build"]);
        assert_eq!(rule.watch_patterns(), vec!["src/**"]);
    }

    #[test]
    fn rejects_service_recovery_in_pure_validation() {
        let mut input = rule_input();
        input.service = true;
        input.recovery = Some(vec![String::from("echo recover")]);

        let error = build_rule(input).expect_err("service recovery is invalid");

        assert!(error.message.contains("cannot declare recovery"));
        assert!(error.hint.is_some());
    }

    #[test]
    fn rejects_manual_lifecycle_conflicts_in_contract_order() {
        let mut input = rule_input();
        input.trigger = Some(TriggerMode::Manual);
        input.run_on_init = true;
        input.service = true;

        let error = build_rule(input).expect_err("manual lifecycle conflict is invalid");

        assert!(error
            .message
            .contains("'trigger: manual' and 'run_on_init'"));
    }

    #[test]
    fn rejects_explicit_timeout_for_services_in_pure_validation() {
        let mut input = rule_input();
        input.service = true;
        input.timeout = Some(Duration::from_secs(30));
        input.timeout_declared = true;

        let error = build_rule(input).expect_err("service timeout is invalid");

        assert!(error.message.contains("'timeout' and 'service: true'"));
    }

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
