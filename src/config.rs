//! Configuration loading: YAML parsing, compatibility formats, and filesystem
//! adapters that turn configuration into task models (`rules::Rules`).
//!
//! Parser DTOs (common-rule groups) and retained YAML presentation live here;
//! the task model in `rules.rs` stays a pure user-facing value with no YAML
//! knowledge. Legacy list, grouped `on`/`tasks`, and nested group formats are
//! all accepted here exactly as documented.

extern crate yaml_rust2;

use crate::cli;
use crate::errors;
use crate::rules::{OutputPolicy, Rules};
use crate::yaml;

use self::yaml_rust2::Yaml;
use self::yaml_rust2::YamlLoader;
use std::fs::File;
use std::io::prelude::*;
use std::time::Duration;
pub fn rule_from(yaml: &Yaml) -> errors::Result<Rules> {
    if yaml["recovery"] != Yaml::BadValue {
        return Err(errors::FzzError::InvalidConfigError(
            "Property 'recovery' is supported only in preferred V2 jobs".to_owned(),
            None,
            Some("Move this task into a `jobs:` configuration before adding recovery.".to_owned()),
        ));
    }
    // MANUAL-TRIGGER-CONTRACT §4: the trigger mode is preferred-V2 only;
    // both legacy parse sites reject it with the same actionable shape as
    // `recovery`.
    if yaml["trigger"] != Yaml::BadValue {
        return Err(errors::FzzError::InvalidConfigError(
            "Property 'trigger' is supported only in preferred V2 jobs".to_owned(),
            None,
            Some("Rename 'tasks' to 'jobs' before declaring a trigger mode.".to_owned()),
        ));
    }
    // FINITE-JOB-TIMEOUT-CONTRACT §1: `timeout` is preferred-V2 only; the
    // legacy root-list site rejects it like `trigger`/`recovery`.
    if yaml["timeout"] != Yaml::BadValue {
        return Err(errors::FzzError::InvalidConfigError(
            "Property 'timeout' is supported only in preferred V2 jobs".to_owned(),
            None,
            Some("Rename 'tasks' to 'jobs' before declaring a timeout.".to_owned()),
        ));
    }
    let name = yaml::extract_string(yaml, "name")?;
    let commands = yaml::extract_list(yaml, "run")?;
    let watch_patterns = ensure_glob_only(
        yaml::extract_list(yaml, "change").unwrap_or_default(),
        "change",
    )?;
    let ignore_patterns = ensure_glob_only(
        yaml::extract_list(yaml, "ignore").unwrap_or_default(),
        "ignore",
    )?;
    let run_on_init = yaml::extract_bool(yaml, "run_on_init");
    let parallel = yaml::extract_optional_string(yaml, "parallel")?;
    let cwd = yaml::extract_optional_string(yaml, "cwd")?;
    let environment = yaml::extract_optional_string_map(yaml, "env")?;

    let rule = Rules::new(name, commands, watch_patterns, ignore_patterns, run_on_init)
        .with_execution_context(cwd, environment);
    Ok(match parallel {
        Some(group) => rule.with_parallel(group),
        None => rule,
    })
}

pub fn commands(rules: Vec<Rules>) -> Vec<String> {
    rules
        .iter()
        .map(|rule| rule.commands())
        .flat_map(|rule| rule.to_vec())
        .collect::<Vec<String>>()
}
pub fn from_yaml(file_content: &str) -> errors::Result<Vec<Rules>> {
    let items = match YamlLoader::load_from_str(file_content) {
        Ok(val) => val,
        Err(err) => {
            let lines: Vec<&str> = file_content.lines().collect();
            let marker = err.marker();

            let line_before = if marker.line() > 1 {
                lines[marker.line() - 2]
            } else {
                ""
            };
            let error_line = if marker.line() > lines.len() {
                lines[lines.len() - 1]
            } else {
                lines[marker.line() - 1]
            };
            let line_after = if marker.line() < lines.len() {
                lines[marker.line()]
            } else {
                ""
            };

            return Err(errors::FzzError::InvalidConfigError(
                format!(
                    "Failed to load configuration at line:\n| {}\n|>{}\n| {}",
                    line_before,
                    error_line,
                    line_after
                ),
                Some(err),
                Some(
                    "Check for wrong types, any missing quotes for glob pattern or incorrect identation".to_owned(),
                ),
            ));
        }
    };

    if items.is_empty() {
        return Err(errors::FzzError::InvalidConfigError(
            "Configuration file is invalid! There are no rules to watch".to_owned(),
            None,
            Some("Make sure to declare at least one rule. Try to run `fzz init` to generate a new configuration from scratch".to_owned()),
        ));
    }

    match &items[0] {
        Yaml::Array(ref items) => {
            let mut rules = vec![];
            for item in items {
                // Check if this item is a group (hash with 'tasks' key)
                match item {
                    Yaml::Hash(_) if item["tasks"] != Yaml::BadValue => {
                        // This is a group with on/tasks format
                        {
                            let group_rules = parse_hash_format(item, true)?;
                            rules.extend(group_rules)
                        }
                    }
                    _ => {
                        // This is a regular task
                        {
                            let rule = rule_from(item)?;
                            rules.push(rule)
                        }
                    }
                }
            }
            Ok(rules)
        },
        Yaml::Hash(ref _hash) => {
            // New format: { on: {...}, tasks: [...] }
            parse_hash_format(&items[0], false)
        },
        other => Err(errors::FzzError::InvalidConfigError(
            format!(
                "Configuration file is invalid. Expected an Array/List of rules got: {}\n```yaml\n{}\n```",
                yaml::get_type(other),
                yaml::yaml_to_string(other, 0),
            ),
            None,
            Some("Make sure to declare the rules as a list without any root property".to_owned()),
        )),
    }
}

/// Represents common rules that can be shared across tasks
struct CommonRules {
    change: Vec<String>,
    ignore: Vec<String>,
    /// Execution-level default output policy, applied to jobs without their
    /// own `output:`.
    output_policy: OutputPolicy,
    timeout: Option<Duration>,
}

fn validate_section(
    root: &Yaml,
    name: &str,
    owner: crate::option_catalog::Owner,
) -> errors::Result<()> {
    let section = &root[name];
    if section == &Yaml::BadValue {
        return Ok(());
    }
    let Yaml::Hash(properties) = section else {
        return Err(errors::FzzError::InvalidConfigError(
            format!("Property '{name}' must be an object"),
            None,
            None,
        ));
    };
    let mut allowed = crate::option_catalog::property_names(owner);
    // Grouped `tasks:` is the still-supported legacy configuration form.
    // Its historical policy locations remain readable only on that boundary;
    // preferred `jobs:` configs must use execution/hooks.
    if owner == crate::option_catalog::Owner::On && root["tasks"] != Yaml::BadValue {
        allowed.extend(["concurrency", "output", "success", "failure", "close"]);
    }
    for (key, _) in properties {
        if let Yaml::String(key) = key {
            if !allowed.contains(&key.as_str()) {
                return Err(errors::FzzError::InvalidConfigError(
                    format!(
                        "Invalid property '{name}.{key}'. Only {} are allowed.",
                        allowed.join(", ")
                    ),
                    None,
                    None,
                ));
            }
        }
    }
    Ok(())
}

fn validate_v2_sections(root: &Yaml) -> errors::Result<()> {
    let Yaml::Hash(properties) = root else {
        return Ok(());
    };
    let mut root_allowed =
        crate::option_catalog::property_names(crate::option_catalog::Owner::Root);
    // `tasks` remains an explicit legacy grouped-form input; it is not part
    // of the V2 schema and is never emitted as preferred configuration.
    root_allowed.push("tasks");
    for (key, _) in properties {
        if let Yaml::String(key) = key {
            if !root_allowed.contains(&key.as_str()) {
                return Err(errors::FzzError::InvalidConfigError(
                    format!(
                        "Invalid property '{key}' at configuration root. Only {} are allowed.",
                        root_allowed.join(", ")
                    ),
                    None,
                    None,
                ));
            }
        }
    }
    for (name, owner) in [
        ("on", crate::option_catalog::Owner::On),
        ("execution", crate::option_catalog::Owner::Execution),
        ("hooks", crate::option_catalog::Owner::Hooks),
    ] {
        validate_section(root, name, owner)?;
    }
    Ok(())
}

fn output_policy_from_root(root: &Yaml) -> errors::Result<OutputPolicy> {
    let execution = &root["execution"];
    let policy = if execution == &Yaml::BadValue && root["tasks"] != Yaml::BadValue {
        &root["on"]
    } else {
        execution
    };
    if policy == &Yaml::BadValue {
        return Ok(OutputPolicy::Inherit);
    }
    match &policy["output"] {
        Yaml::BadValue => Ok(OutputPolicy::Inherit),
        Yaml::String(raw) => match raw.as_str() {
            "inherit" => Ok(OutputPolicy::Inherit),
            "quiet" => Ok(OutputPolicy::Quiet),
            "capture" => Ok(OutputPolicy::Capture),
            "show-on-failure" => Ok(OutputPolicy::ShowOnFailure),
            _ => Err(errors::FzzError::InvalidConfigError(
                format!("Property 'execution.output' has invalid value '{raw}': expected inherit, quiet, capture, or show-on-failure"), None, None,
            )),
        },
        _ => Err(errors::FzzError::InvalidConfigError("Property 'execution.output' must be a string".to_owned(), None, None)),
    }
}

/// Parse the grouped root format: `{ on: {...}, jobs: [...] }` (preferred V2,
/// TASK-0075) or the explicitly accepted legacy `tasks:` spelling. Mixed keys
/// are an error, never a silent merge; jobs must be an ordered list.
fn parse_hash_format(yaml: &Yaml, allow_empty: bool) -> errors::Result<Vec<Rules>> {
    let has_jobs = yaml["jobs"] != Yaml::BadValue;
    let has_tasks = yaml["tasks"] != Yaml::BadValue;
    if has_jobs && has_tasks {
        return Err(errors::FzzError::InvalidConfigError(
            "Configuration file is invalid: use exactly one of 'jobs' (preferred) or 'tasks' (compatibility), not both".to_owned(),
            None,
            Some("Example:\non:\n  change: [\"src/**\"]\njobs:\n  - name: build\n    run: cargo build".to_owned()),
        ));
    }
    let key = if has_jobs { "jobs" } else { "tasks" };

    if has_tasks && yaml["execution"]["timeout"] != Yaml::BadValue {
        return Err(errors::FzzError::InvalidConfigError(
            "Property 'execution.timeout' is supported only in preferred V2 jobs".to_owned(),
            None,
            None,
        ));
    }
    if has_tasks && yaml["execution"]["recovery_policy"] != Yaml::BadValue {
        return Err(errors::FzzError::InvalidConfigError(
            "Property 'execution.recovery_policy' is supported only in preferred V2 jobs"
                .to_owned(),
            None,
            Some("Rename 'tasks' to 'jobs' before configuring recovery policy.".to_owned()),
        ));
    }

    // Extract the jobs/tasks array (ordered list; mapping form is rejected so
    // declaration order can never be reordered implicitly).
    let tasks_yaml = &yaml[key];
    let tasks_array = match tasks_yaml {
        Yaml::Array(ref items) => items,
        Yaml::BadValue => {
            return Err(errors::FzzError::InvalidConfigError(
                format!(
                    "Configuration file is invalid. When using the 'on' format, you must provide a '{}' array",
                    key
                ),
                None,
                Some("Example:\non:\n  change: [\"src/**\"]\njobs:\n  - name: build\n    run: cargo build".to_owned()),
            ));
        }
        Yaml::Hash(_) => {
            return Err(errors::FzzError::InvalidConfigError(
                format!(
                    "Configuration file is invalid. '{}' must be an ordered list, not a mapping — Funzzy job order is semantic (barriers and group occurrences derive from declaration order)",
                    key
                ),
                None,
                Some("Example:\non:\n  change: [\"src/**\"]\njobs:\n  - name: build\n    run: cargo build".to_owned()),
            ));
        }
        _ => {
            return Err(errors::FzzError::InvalidConfigError(
                format!(
                    "Configuration file is invalid. '{}' must be an Array/List, got: {}\n```yaml\n{}\n```",
                    key,
                    yaml::get_type(tasks_yaml),
                    yaml::yaml_to_string(tasks_yaml, 0),
                ),
                None,
                Some("Make sure '{}' is defined as a list of job objects".to_owned()),
            ));
        }
    };

    if tasks_array.is_empty() && !allow_empty {
        return Err(errors::FzzError::InvalidConfigError(
            format!("Configuration file is invalid. '{}' cannot be empty", key),
            None,
            Some("Add at least one job with a name and run command".to_owned()),
        ));
    }

    validate_v2_sections(yaml)?;
    recovery_policy_from_root(yaml)
        .map_err(|error| errors::FzzError::InvalidConfigError(error, None, None))?;

    // Extract common rules from the 'on' section (optional). Execution policy
    // has its own V2 owner and is inherited by jobs that omit `output`.
    let mut common_rules = extract_common_rules(&yaml["on"])?;
    common_rules.output_policy = output_policy_from_root(yaml)?;
    common_rules.timeout = execution_timeout_from_root(yaml)?;

    // Parse each task and merge with common rules; duplicate names are a
    // config bug (TASK-0075/0076), never a silent merge or reorder.
    let mut rules = vec![];
    let mut seen: std::collections::HashSet<String> = std::collections::HashSet::new();
    for task_yaml in tasks_array {
        if has_tasks && task_yaml["recovery"] != Yaml::BadValue {
            return Err(errors::FzzError::InvalidConfigError(
                "Property 'recovery' is supported only in preferred V2 jobs".to_owned(),
                None,
                Some("Rename 'tasks' to 'jobs' before declaring a recovery.".to_owned()),
            ));
        }
        // MANUAL-TRIGGER-CONTRACT §4: grouped legacy `tasks:` entries reject
        // `trigger` like `recovery` (second legacy parse site).
        if has_tasks && task_yaml["trigger"] != Yaml::BadValue {
            return Err(errors::FzzError::InvalidConfigError(
                "Property 'trigger' is supported only in preferred V2 jobs".to_owned(),
                None,
                Some("Rename 'tasks' to 'jobs' before declaring a trigger mode.".to_owned()),
            ));
        }
        // FINITE-JOB-TIMEOUT-CONTRACT §1: grouped legacy `tasks:` entries
        // reject `timeout` at the second legacy parse site.
        if has_tasks && task_yaml["timeout"] != Yaml::BadValue {
            return Err(errors::FzzError::InvalidConfigError(
                "Property 'timeout' is supported only in preferred V2 jobs".to_owned(),
                None,
                Some("Rename 'tasks' to 'jobs' before declaring a timeout.".to_owned()),
            ));
        }
        let rule = rule_from_with_common(task_yaml, &common_rules)?;
        if !seen.insert(rule.name.clone()) {
            return Err(errors::FzzError::InvalidConfigError(
                format!(
                    "Configuration file is invalid. Duplicate {} name '{}'",
                    key.trim_end_matches('s'),
                    rule.name
                ),
                None,
                Some("Each job needs a unique name; rename one of the duplicates".to_owned()),
            ));
        }
        rules.push(rule);
    }

    Ok(rules)
}

/// Extract common change and ignore patterns from the 'on' section
fn extract_common_rules(yaml: &Yaml) -> errors::Result<CommonRules> {
    match yaml {
        Yaml::BadValue => {
            // No 'on' section, return empty common rules
            Ok(CommonRules {
                change: vec![],
                ignore: vec![],
                output_policy: OutputPolicy::Inherit,
                timeout: None,
            })
        }
        Yaml::Hash(_) => {
            let change = yaml::extract_list(yaml, "change").unwrap_or_default();
            let ignore = yaml::extract_list(yaml, "ignore").unwrap_or_default();

            Ok(CommonRules {
                change: ensure_glob_only(change, "on.change")?,
                ignore: ensure_glob_only(ignore, "on.ignore")?,
                output_policy: OutputPolicy::Inherit,
                timeout: None,
            })
        }
        _ => Err(errors::FzzError::InvalidConfigError(
            format!(
                "Configuration file is invalid. 'on' must be a Hash/Object, got: {}\n```yaml\n{}\n```",
                yaml::get_type(yaml),
                yaml::yaml_to_string(yaml, 0),
            ),
            None,
            Some("Example:\non:\n  change: [\"src/**\"]\n  ignore: [\"**/*.log\"]".to_owned()),
        )),
    }
}

/// Parse a rule from YAML and merge with common rules
fn rule_from_with_common(yaml: &Yaml, common: &CommonRules) -> errors::Result<Rules> {
    // Job-level allowlist from the canonical catalog (TASK-0094): an unknown
    // property is an actionable config bug, never a silent accept
    // (JOBS-CONFIG-CONTRACT §5, schema `additionalProperties: false`).
    if let Yaml::Hash(ref hash) = yaml {
        let allowed = crate::option_catalog::property_names(crate::option_catalog::Owner::Job);
        for (key, _) in hash {
            if let Yaml::String(ref key_str) = key {
                if !allowed.contains(&key_str.as_str()) {
                    let name =
                        yaml::extract_string(yaml, "name").unwrap_or_else(|_| "?".to_owned());
                    return Err(errors::FzzError::InvalidConfigError(
                        format!(
                            "Invalid property '{}' in job '{}'. Only {} are allowed.",
                            key_str,
                            name,
                            allowed.join(", ")
                        ),
                        None,
                        Some("Each job needs a unique name and a run command; check the property spelling".to_owned()),
                    ));
                }
            }
        }
    }

    let name = yaml::extract_string(yaml, "name")?;
    let commands = yaml::extract_list(yaml, "run")?;

    // Tasks EXTEND the shared `on` rules; they never replace them. A task's
    // own `change` and `ignore` are appended to (and deduped against) the
    // common patterns, so root-level scope and safety rails always apply.
    let task_change = yaml::extract_list(yaml, "change").unwrap_or_default();
    let task_ignore = yaml::extract_list(yaml, "ignore").unwrap_or_default();
    let manual = match &yaml["trigger"] {
        Yaml::BadValue => false,
        Yaml::String(raw) if raw == "manual" => true,
        Yaml::String(_) => {
            return Err(errors::FzzError::InvalidConfigError(
                format!(
                    "Invalid 'trigger' value for job '{}': must be one of: manual",
                    name
                ),
                None,
                Some(
                    "Use `trigger: manual` for explicit-run-only jobs, or remove 'trigger'."
                        .to_owned(),
                ),
            ));
        }
        _ => {
            return Err(errors::FzzError::InvalidConfigError(
                format!(
                    "Invalid 'trigger' value for job '{}': must be the string 'manual'",
                    name
                ),
                None,
                None,
            ));
        }
    };
    // MANUAL-TRIGGER-CONTRACT §4: ambiguous combinations are actionable
    // errors, never silent precedence.
    if manual {
        if !task_change.is_empty() {
            return Err(errors::FzzError::InvalidConfigError(
                format!("Job '{}' declares both 'trigger: manual' and 'change'", name),
                None,
                Some("Manual jobs never match filesystem events; remove 'change' or 'trigger: manual'.".to_owned()),
            ));
        }
        if !task_ignore.is_empty() {
            return Err(errors::FzzError::InvalidConfigError(
                format!(
                    "Job '{}' declares both 'trigger: manual' and 'ignore'",
                    name
                ),
                None,
                Some(
                    "'ignore' is inert on a manual job; remove 'ignore' or 'trigger: manual'."
                        .to_owned(),
                ),
            ));
        }
    }

    let (watch_patterns, ignore_patterns) = if manual {
        // MANUAL-TRIGGER-CONTRACT §3.1: root `on` scope never applies to a
        // manual job — its effective watch/ignore surface is empty.
        (vec![], vec![])
    } else {
        (
            ensure_glob_only(merge_patterns(&common.change, task_change), "change")?,
            ensure_glob_only(merge_patterns(&common.ignore, task_ignore), "ignore")?,
        )
    };
    let run_on_init = yaml::extract_bool(yaml, "run_on_init");
    let trigger = manual.then_some(crate::rules::TriggerMode::Manual);
    let parallel = yaml::extract_optional_string(yaml, "parallel")?;
    let cwd = yaml::extract_optional_string(yaml, "cwd")?;
    let environment = yaml::extract_optional_string_map(yaml, "env")?;
    let recovery = recovery_commands_from_yaml(yaml, &name)?;
    // Strict: `service` must be a boolean when present (TASK-0035); a typo
    // like `yes` must not silently disable service management.
    let service = match &yaml["service"] {
        Yaml::BadValue => false,
        Yaml::Boolean(value) => *value,
        _ => {
            return Err(errors::FzzError::InvalidConfigError(
                format!(
                    "Invalid 'service' value for job '{}': must be a boolean",
                    name
                ),
                None,
                None,
            ))
        }
    };
    if service && recovery.is_some() {
        return Err(errors::FzzError::InvalidConfigError(
            format!(
                "Job '{}' cannot declare recovery when service is true",
                name
            ),
            None,
            Some(
                "A service has no finite verification boundary; remove `recovery` or `service`."
                    .to_owned(),
            ),
        ));
    }
    // MANUAL-TRIGGER-CONTRACT §4: reject ambiguous manual combinations with
    // actionable errors rather than inventing precedence.
    if trigger.is_some() && run_on_init {
        return Err(errors::FzzError::InvalidConfigError(
            format!(
                "Job '{}' cannot declare both 'trigger: manual' and 'run_on_init'",
                name
            ),
            None,
            Some(
                "Manual jobs never run at watcher initialization; remove 'run_on_init' or 'trigger: manual'."
                    .to_owned(),
            ),
        ));
    }
    if trigger.is_some() && service {
        return Err(errors::FzzError::InvalidConfigError(
            format!(
                "Job '{}' cannot declare both 'trigger: manual' and 'service: true'",
                name
            ),
            None,
            Some(
                "Services start on init and restart on change; that contradicts 'trigger: manual'. Remove one."
                    .to_owned(),
            ),
        ));
    }
    // FINITE-JOB-TIMEOUT-CONTRACT §7: a managed service is intentionally
    // unbounded; a finite deadline contradicts the service contract.
    let timeout = match &yaml["timeout"] {
        Yaml::BadValue if common.timeout.is_some() => common.timeout,
        Yaml::BadValue => None,
        Yaml::String(raw) => parse_duration("timeout", raw).map_err(|error| {
            errors::FzzError::InvalidConfigError(
                format!("Invalid 'timeout' value for job '{}': {error}", name),
                None,
                Some(
                    "Use a positive duration with ms/s/m units, e.g. `timeout: 30m` (bare number = seconds)."
                        .to_owned(),
                ),
            )
        })?,
        // A bare YAML integer is seconds, same grammar as the string form
        // (FINITE-JOB-TIMEOUT-CONTRACT §1: bare number = seconds).
        Yaml::Integer(seconds) => parse_duration("timeout", &format!("{seconds}s")).map_err(
            |error| {
                errors::FzzError::InvalidConfigError(
                    format!("Invalid 'timeout' value for job '{}': {error}", name),
                    None,
                    Some(
                        "Use a positive duration with ms/s/m units, e.g. `timeout: 30m` (bare number = seconds)."
                            .to_owned(),
                    ),
                )
            },
        )?,
        _ => {
            return Err(errors::FzzError::InvalidConfigError(
                format!(
                    "Invalid 'timeout' value for job '{}': must be a duration string (e.g. 30m, 45s, 200ms)",
                    name
                ),
                None,
                None,
            ));
        }
    };
    if yaml["timeout"] != Yaml::BadValue && timeout.is_some() && service {
        return Err(errors::FzzError::InvalidConfigError(
            format!(
                "Job '{}' cannot declare both 'timeout' and 'service: true'",
                name
            ),
            None,
            Some("A service is intentionally unbounded; remove 'timeout' or 'service'.".to_owned()),
        ));
    }
    let timeout = if service { None } else { timeout };

    let output = match yaml::extract_optional_string(yaml, "output")? {
        None => common.output_policy,
        Some(raw) => match raw.as_str() {
            "inherit" => OutputPolicy::Inherit,
            "quiet" => OutputPolicy::Quiet,
            "capture" => OutputPolicy::Capture,
            "show-on-failure" => OutputPolicy::ShowOnFailure,
            other => {
                return Err(errors::FzzError::InvalidConfigError(
                    format!(
                        "Invalid output policy '{}' for job '{}': expected inherit, quiet, capture, or show-on-failure",
                        other, name
                    ),
                    None,
                    None,
                ))
            }
        },
    };

    let rule = Rules::new(name, commands, watch_patterns, ignore_patterns, run_on_init)
        .with_execution_context(cwd, environment)
        .with_inherited_patterns(inherited_patterns(common))
        .with_output(output)
        .with_service(service)
        .with_trigger(trigger)
        .with_timeout(timeout);
    let rule = match recovery {
        Some(commands) => rule.with_recovery(commands),
        None => rule,
    };
    Ok(match parallel {
        Some(group) => rule.with_parallel(group),
        None => rule,
    })
}

fn execution_timeout_from_root(root: &Yaml) -> errors::Result<Option<Duration>> {
    let execution = &root["execution"];
    if execution == &Yaml::BadValue || execution["timeout"] == Yaml::BadValue {
        return Ok(None);
    }
    let raw = match &execution["timeout"] {
        Yaml::String(value) => value.clone(),
        Yaml::Integer(seconds) => format!("{seconds}s"),
        _ => {
            return Err(errors::FzzError::InvalidConfigError(
                "Property 'execution.timeout' must be a positive duration string or number"
                    .to_owned(),
                None,
                None,
            ))
        }
    };
    parse_duration("execution.timeout", &raw)
        .map_err(|error| errors::FzzError::InvalidConfigError(error, None, None))
}

fn recovery_commands_from_yaml(yaml: &Yaml, name: &str) -> errors::Result<Option<Vec<String>>> {
    match &yaml["recovery"] {
        Yaml::BadValue => Ok(None),
        Yaml::String(command) if command.trim().is_empty() => {
            Err(errors::FzzError::InvalidConfigError(
                format!("Job '{}' recovery must be a non-empty command", name),
                None,
                None,
            ))
        }
        Yaml::String(command) => Ok(Some(vec![command.clone()])),
        Yaml::Array(commands) if commands.is_empty() => Err(errors::FzzError::InvalidConfigError(
            format!("Job '{}' recovery must contain at least one command", name),
            None,
            None,
        )),
        Yaml::Array(commands) => {
            let mut parsed = Vec::with_capacity(commands.len());
            for (index, command) in commands.iter().enumerate() {
                let Some(command) = command.as_str() else {
                    return Err(errors::FzzError::InvalidConfigError(
                        format!(
                            "Job '{}' recovery command {} must be a string",
                            name,
                            index + 1
                        ),
                        None,
                        None,
                    ));
                };
                if command.trim().is_empty() {
                    return Err(errors::FzzError::InvalidConfigError(
                        format!(
                            "Job '{}' recovery command {} must be non-empty",
                            name,
                            index + 1
                        ),
                        None,
                        None,
                    ));
                }
                parsed.push(command.to_owned());
            }
            Ok(Some(parsed))
        }
        _ => Err(errors::FzzError::InvalidConfigError(
            format!(
                "Job '{}' recovery must be a command string or ordered string list",
                name
            ),
            None,
            None,
        )),
    }
}

/// Append task-specific patterns to the common ones, dropping duplicates.
/// Common patterns keep their position so merged output stays stable.
fn merge_patterns(common: &[String], task: Vec<String>) -> Vec<String> {
    let mut merged: Vec<String> = common.to_vec();
    for pattern in task {
        if !merged.contains(&pattern) {
            merged.push(pattern);
        }
    }
    merged
}

/// The group-provided patterns (change + ignore) that tasks inherit. Used to
/// mark `rule_origin=group` in diagnostics when one of these patterns is the
/// effective rule for a decision.
fn inherited_patterns(common: &CommonRules) -> Vec<String> {
    let mut inherited = common.change.clone();
    for pattern in &common.ignore {
        if !inherited.contains(pattern) {
            inherited.push(pattern.clone());
        }
    }
    inherited
}

fn ensure_glob_only(patterns: Vec<String>, field_name: &str) -> errors::Result<Vec<String>> {
    for pattern in &patterns {
        let trimmed = pattern.trim_start();
        if trimmed == ":lua" || trimmed.starts_with(":lua ") {
            return Err(errors::FzzError::InvalidConfigError(
                format!(
                    "Property '{}' no longer accepts ':lua' entries. Only glob patterns are supported.",
                    field_name
                ),
                None,
                Some("Remove ':lua' entries and use plain glob patterns instead.".to_owned()),
            ));
        }
    }

    Ok(patterns)
}

fn prepare_as_glob_pattern(line: &str) -> errors::Result<String> {
    let current_dir = match std::env::current_dir() {
        Ok(val) => val,
        Err(err) => {
            return Err(errors::FzzError::IoConfigError(
                "Failed to get current directory".to_owned(),
                Some(err),
            ));
        }
    };

    let path = std::path::Path::new(&line);

    let full_path = if path.starts_with(".") {
        if line.len() == 1 {
            current_dir.join("")
        } else {
            current_dir.join(&line[2..])
        }
    } else {
        current_dir.join(line)
    };

    if full_path.is_dir() {
        let full_path_as_str = match full_path.join("**").to_str() {
            Some(val) => val.to_owned(),
            _ => {
                return Err(errors::FzzError::PathPatternError(
                    format!(
                        "Failed to convert path '{:?}' to a recursive glob pattern.",
                        full_path
                    ),
                    None,
                ))
            }
        };

        return Ok(full_path_as_str);
    }

    match full_path.to_str() {
        Some(val) => Ok(val.to_owned()),
        _ => Err(errors::FzzError::PathPatternError(
            format!("Failed to convert path '{:?}' to string.", full_path),
            None,
        )),
    }
}
pub fn extract_paths(stdinput: String) -> errors::Result<Vec<String>> {
    let mut watches = vec![];
    let mut line_number = 0;
    for pathline in stdinput.lines() {
        line_number += 1;
        let path = std::path::Path::new(&pathline);

        match path.canonicalize() {
            Ok(val) => {
                watches.push(val.to_str().unwrap().to_owned());
            }
            Err(err) => {
                return Err(errors::FzzError::PathError(
                    format!("Unknown path '{}' at line {}", path.to_str().unwrap(), line_number),
                    Some(errors::UnkownError::from(err)),
                    Some(
                        ["When using stdin, make sure to provide a list of valid files or directories.",
                        "The output of command `find` is a good example"].join("\n"),
                    ),
                ));
            }
        }
    }

    Ok(watches)
}

/// Builds an ad-hoc `exec` rule from stdin patterns and an exact argv. The
/// argv crosses the parser/runtime boundary without being joined/re-parsed.
pub fn from_argv(patterns: Vec<String>, argv: Vec<String>) -> errors::Result<Vec<Rules>> {
    let watches = patterns
        .iter()
        .map(|pathline| prepare_as_glob_pattern(pathline))
        .collect::<errors::Result<Vec<String>>>()?;

    let run_on_init = true;
    let ignore = vec![];
    Ok(vec![Rules::from_argv(
        "unnamed".to_owned(),
        argv,
        watches,
        ignore,
        run_on_init,
    )])
}
pub fn control_socket_from_yaml(content: &str) -> Result<Option<String>, String> {
    let documents = YamlLoader::load_from_str(content).map_err(|err| err.to_string())?;
    let root = documents
        .first()
        .ok_or_else(|| "Configuration file is empty".to_owned())?;
    let on = &root["on"];

    if on == &Yaml::BadValue {
        return Ok(None);
    }

    if !matches!(on, Yaml::Hash(_)) {
        return Err("Property 'on' must be an object".to_owned());
    }

    match &on["socket"] {
        Yaml::BadValue => Ok(None),
        Yaml::String(path) if !path.trim().is_empty() => Ok(Some(path.to_owned())),
        Yaml::String(_) => Err("Property 'on.socket' cannot be empty".to_owned()),
        _ => Err("Property 'on.socket' must be a string".to_owned()),
    }
}

pub fn control_socket_from_file(filename: &str) -> Result<Option<String>, String> {
    let mut file = File::open(filename).map_err(|err| err.to_string())?;
    let mut content = String::new();
    file.read_to_string(&mut content)
        .map_err(|err| err.to_string())?;
    control_socket_from_yaml(&content)
}

/// Policy for whether a failed job's configured recovery may be offered.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum RecoveryPolicy {
    Prompt,
    Skip,
}

/// Reads `execution.recovery_policy`. Preferred V2 defaults to `prompt`;
/// legacy task-list inputs cannot opt into this policy through YAML.
pub fn recovery_policy_from_yaml(content: &str) -> Result<RecoveryPolicy, String> {
    recovery_policy_from_yaml_with_default(content, RecoveryPolicy::Prompt)
}

/// Parses the recovery policy while preserving a caller's startup default when
/// the candidate omits the policy entirely (used by live reload).
pub fn recovery_policy_from_yaml_with_default(
    content: &str,
    default: RecoveryPolicy,
) -> Result<RecoveryPolicy, String> {
    let documents = YamlLoader::load_from_str(content).map_err(|err| err.to_string())?;
    let root = documents
        .first()
        .ok_or_else(|| "Configuration file is empty".to_owned())?;
    if root["execution"] == Yaml::BadValue || root["execution"]["recovery_policy"] == Yaml::BadValue
    {
        return Ok(default);
    }
    recovery_policy_from_root(root)
}

fn recovery_policy_from_root(root: &Yaml) -> Result<RecoveryPolicy, String> {
    let execution = &root["execution"];
    if execution == &Yaml::BadValue {
        return Ok(RecoveryPolicy::Prompt);
    }
    if root["tasks"] != Yaml::BadValue && execution["recovery_policy"] != Yaml::BadValue {
        return Err(
            "Property 'execution.recovery_policy' is supported only in preferred V2 jobs"
                .to_owned(),
        );
    }
    if !matches!(execution, Yaml::Hash(_)) {
        return Err("Property 'execution' must be an object".to_owned());
    }
    match &execution["recovery_policy"] {
        Yaml::BadValue => Ok(RecoveryPolicy::Prompt),
        Yaml::String(value) => match value.as_str() {
            "prompt" => Ok(RecoveryPolicy::Prompt),
            "skip" => Ok(RecoveryPolicy::Skip),
            other => Err(format!(
                "Property 'execution.recovery_policy' has invalid value '{}': expected prompt or skip",
                other
            )),
        },
        _ => Err("Property 'execution.recovery_policy' must be prompt or skip".to_owned()),
    }
}

pub fn recovery_policy_from_file(filename: &str) -> Result<RecoveryPolicy, String> {
    let mut file = File::open(filename).map_err(|err| err.to_string())?;
    let mut content = String::new();
    file.read_to_string(&mut content)
        .map_err(|err| err.to_string())?;
    recovery_policy_from_yaml(&content)
}

/// Parses the approval-only recovery timeout. Missing values preserve the
/// caller's effective default so reloads can freeze the prior policy.
pub fn recovery_timeout_from_yaml_with_default(
    content: &str,
    default: Duration,
) -> Result<Duration, String> {
    let documents = YamlLoader::load_from_str(content).map_err(|err| err.to_string())?;
    let root = documents
        .first()
        .ok_or_else(|| "Configuration file is empty".to_owned())?;
    let execution = &root["execution"];
    if *execution == Yaml::BadValue || execution["recovery_timeout"] == Yaml::BadValue {
        return Ok(default);
    }
    if !matches!(execution, Yaml::Hash(_)) {
        return Err("Property 'execution' must be an object".to_owned());
    }
    let raw = match &execution["recovery_timeout"] {
        Yaml::Integer(value) => value.to_string(),
        Yaml::String(value) => value.clone(),
        _ => {
            return Err(
                "Property 'execution.recovery_timeout' must be a duration string or number"
                    .to_owned(),
            )
        }
    };
    parse_recovery_timeout(&raw)
}

pub fn recovery_timeout_from_yaml(content: &str) -> Result<Duration, String> {
    recovery_timeout_from_yaml_with_default(content, Duration::from_secs(60))
}

fn parse_recovery_timeout(raw: &str) -> Result<Duration, String> {
    parse_debounce(raw)?
        .ok_or_else(|| "recovery timeout cannot be empty".to_owned())
        .map_err(|error| error.replace("'on.debounce'", "'execution.recovery_timeout'"))
}

pub fn concurrency_from_yaml(content: &str) -> Result<Option<usize>, String> {
    let documents = YamlLoader::load_from_str(content).map_err(|err| err.to_string())?;
    let root = documents
        .first()
        .ok_or_else(|| "Configuration file is empty".to_owned())?;
    let execution = &root["execution"];
    let policy = if execution == &Yaml::BadValue && root["tasks"] != Yaml::BadValue {
        &root["on"]
    } else {
        execution
    };
    if policy == &Yaml::BadValue {
        return Ok(None);
    }
    if !matches!(policy, Yaml::Hash(_)) {
        return Err("Property 'execution' must be an object".to_owned());
    }
    match &policy["concurrency"] {
        Yaml::BadValue => Ok(None),
        Yaml::Integer(value) if *value > 0 => Ok(Some(*value as usize)),
        Yaml::Integer(_) => Err(
            "Property 'execution.concurrency' must be a positive integer (got zero or negative)"
                .to_owned(),
        ),
        _ => Err("Property 'execution.concurrency' must be a positive integer".to_owned()),
    }
}

pub fn concurrency_from_file(filename: &str) -> Result<Option<usize>, String> {
    let mut file = File::open(filename).map_err(|err| err.to_string())?;
    let mut content = String::new();
    file.read_to_string(&mut content)
        .map_err(|err| err.to_string())?;
    concurrency_from_yaml(&content)
}

/// Parses the optional `on.debounce` duration. Bare numbers are seconds;
/// `ms`/`s`/`m` suffixes are accepted. Absent defaults to the existing
/// one-second behavior (contract keeps it unless explicitly configured);
/// zero and invalid values are rejected.
pub fn debounce_from_yaml(content: &str) -> Result<Option<Duration>, String> {
    let documents = YamlLoader::load_from_str(content).map_err(|err| err.to_string())?;
    let root = documents
        .first()
        .ok_or_else(|| "Configuration file is empty".to_owned())?;
    let on = &root["on"];

    if on == &Yaml::BadValue {
        return Ok(None);
    }

    if !matches!(on, Yaml::Hash(_)) {
        return Err("Property 'on' must be an object".to_owned());
    }

    if on["debounce"] == Yaml::BadValue {
        return Ok(None);
    }

    let raw = match &on["debounce"] {
        Yaml::Integer(value) => value.to_string(),
        Yaml::String(value) => value.clone(),
        _ => return Err("Property 'on.debounce' must be a duration string or number".to_owned()),
    };
    parse_debounce(&raw)
}

/// Parses one debounce duration: `<number>` (seconds), or `<number>ms|s|m`.
/// Rejects zero and unknown suffixes so a typo never silently changes timing.
pub fn parse_debounce(raw: &str) -> Result<Option<Duration>, String> {
    parse_duration("on.debounce", raw)
}

/// Shared duration grammar (FINITE-JOB-TIMEOUT-CONTRACT §1, same as
/// `on.debounce`): `<number>` with optional `ms`/`s`/`m` suffix; a bare
/// number means seconds; strictly positive; hours/composite are NOT
/// accepted. TASK-0139 must not extend this parser.
pub fn parse_duration(field: &str, raw: &str) -> Result<Option<Duration>, String> {
    let raw = raw.trim();
    let (digits, multiplier) = if let Some(stripped) = raw.strip_suffix("ms") {
        (stripped, 1u64)
    } else if let Some(stripped) = raw.strip_suffix('s') {
        (stripped, 1_000u64)
    } else if let Some(stripped) = raw.strip_suffix('m') {
        (stripped, 60_000u64)
    } else {
        (raw, 1_000u64)
    };
    let value: u64 = digits.trim().parse().map_err(|_| {
        format!(
            "invalid '{field}' duration '{raw}': expected <number> with optional ms/s/m suffix (bare number = seconds)"
        )
    })?;
    if value == 0 {
        return Err(format!(
            "invalid '{field}' duration '{raw}': must be positive"
        ));
    }
    let millis = value
        .checked_mul(multiplier)
        .ok_or_else(|| format!("invalid '{field}' duration '{raw}': too large"))?;
    Ok(Some(Duration::from_millis(millis)))
}

pub fn debounce_from_file(filename: &str) -> Result<Option<Duration>, String> {
    let mut file = File::open(filename).map_err(|err| err.to_string())?;
    let mut content = String::new();
    file.read_to_string(&mut content)
        .map_err(|err| err.to_string())?;
    debounce_from_yaml(&content)
}

pub fn from_file(filename: &str) -> errors::Result<Vec<Rules>> {
    match File::open(filename) {
        Ok(mut file) => {
            let mut content = String::new();

            if let Err(err) = file.read_to_string(&mut content) {
                return Err(errors::FzzError::IoConfigError(
                    format!("Couldn't read configuration file: '{}'", filename),
                    Some(err),
                ));
            }

            from_yaml(&content)
        }

        Err(err) => Err(errors::FzzError::IoConfigError(
            format!("Couldn't open configuration file: '{}'", filename),
            Some(err),
        )),
    }
}

pub fn from_default_file_config() -> errors::Result<Vec<Rules>> {
    let default_filename = cli::watch::DEFAULT_FILENAME;
    match from_file(default_filename) {
        Ok(rules) => Ok(rules),
        Err(err) => match from_file(&default_filename.replace(".yaml", ".yml")) {
            Ok(rules) => Ok(rules),
            Err(_) => Err(err),
        },
    }
}

/// Renders a task back to YAML for presentation (verbose logs).
///
/// The task model does not retain raw YAML; the render is reconstructed from
/// model fields, so only canonical scalar/list forms are produced.
pub fn rule_as_yaml(rule: &Rules) -> String {
    let mut lines = vec![format!("name: {}", rule.name)];

    lines.push(render_scalar_or_list("run", &rule.commands()));
    if let Some(recovery) = rule.recovery_commands() {
        lines.push(render_scalar_or_list("recovery", &recovery));
    }
    if let Some(trigger) = rule.trigger() {
        // MANUAL-TRIGGER-CONTRACT §6/QA P2: a rendered manual job has no
        // change/init surface; dropping `trigger` would make the render an
        // invalid config (silent trigger loss).
        lines.push(format!("trigger: {}", trigger.as_str()));
    }
    if let Some(timeout) = rule.timeout() {
        // FINITE-JOB-TIMEOUT-CONTRACT §1/QA P2: a rendered bounded job must
        // stay bounded — dropping `timeout` would silently unbound it.
        lines.push(format!("timeout: {}", render_duration(timeout)));
    }
    if !rule.watch_patterns().is_empty() {
        lines.push(render_scalar_or_list("change", &rule.watch_patterns()));
    }
    if !rule.ignore_glob_patterns().is_empty() {
        lines.push(render_scalar_or_list(
            "ignore",
            &rule.ignore_glob_patterns(),
        ));
    }
    if rule.run_on_init() {
        lines.push("run_on_init: true".to_owned());
    }

    lines.join("\n")
}

/// Canonical duration text (m > s > ms, largest unit that divides evenly);
/// always round-trips through `parse_duration` (FINITE-JOB-TIMEOUT-CONTRACT §1).
fn render_duration(duration: Duration) -> String {
    let ms = duration.as_millis();
    if ms >= 60_000 && ms.is_multiple_of(60_000) {
        format!("{}m", ms / 60_000)
    } else if ms >= 1_000 && ms.is_multiple_of(1_000) {
        format!("{}s", ms / 1_000)
    } else {
        format!("{}ms", ms)
    }
}

fn render_scalar_or_list(prop: &str, values: &[String]) -> String {
    if values.len() == 1 {
        return format!("{}: {}", prop, values[0]);
    }

    let items = values
        .iter()
        .map(|value| format!("  - {}", value))
        .collect::<Vec<String>>()
        .join("\n");
    format!("{}:\n{}", prop, items)
}

pub fn format_rules(rule: &Vec<Rules>) -> String {
    let mut formatted_rules = String::new();

    for rule in rule {
        formatted_rules.push_str(&format!("{}\n", rule_as_yaml(rule)));
    }

    formatted_rules
}

#[cfg(test)]
mod tests {
    extern crate yaml_rust2;

    use self::yaml_rust2::YamlLoader;
    use super::concurrency_from_yaml;
    use super::control_socket_from_yaml;
    use super::from_argv;
    use super::from_yaml;
    use super::rule_as_yaml;
    use super::rule_from;
    use std::env::current_dir;

    #[test]
    fn recovery_policy_defaults_to_prompt_and_accepts_skip() {
        assert_eq!(
            super::recovery_policy_from_yaml("jobs: []\n").unwrap(),
            super::RecoveryPolicy::Prompt
        );
        assert_eq!(
            super::recovery_policy_from_yaml("execution:\n  recovery_policy: skip\njobs: []\n")
                .unwrap(),
            super::RecoveryPolicy::Skip
        );
    }

    #[test]
    fn recovery_timeout_defaults_to_sixty_seconds_and_accepts_duration_syntax() {
        assert_eq!(
            super::recovery_timeout_from_yaml("jobs: []\n").unwrap(),
            std::time::Duration::from_secs(60)
        );
        assert_eq!(
            super::recovery_timeout_from_yaml("execution:\n  recovery_timeout: 250ms\njobs: []\n")
                .unwrap(),
            std::time::Duration::from_millis(250)
        );
    }

    #[test]
    fn recovery_timeout_rejects_zero_and_invalid_values() {
        for value in ["0", "fast", "1h"] {
            let yaml = format!("execution:\n  recovery_timeout: {value}\njobs: []\n");
            assert!(super::recovery_timeout_from_yaml(&yaml).is_err(), "{value}");
        }
    }

    #[test]
    fn recovery_policy_rejects_invalid_values() {
        for value in ["true", "auto", "always", "never"] {
            let yaml = format!("execution:\n  recovery_policy: {value}\njobs: []\n");
            assert!(super::recovery_policy_from_yaml(&yaml).is_err(), "{value}");
        }
    }

    #[test]
    fn it_reads_control_socket_from_on_config() {
        let file_content = r#"
on:
  socket: .tmp/funzzy/control.sock
tasks:
  - name: my tests
    run: cargo test
    run_on_init: true
"#;

        assert_eq!(
            control_socket_from_yaml(file_content).unwrap(),
            Some(".tmp/funzzy/control.sock".to_owned())
        );
        assert!(from_yaml(file_content).is_ok());
    }

    #[test]
    fn it_keeps_control_socket_optional_for_legacy_config() {
        let file_content = r#"
- name: my tests
  run: cargo test
  run_on_init: true
"#;

        assert_eq!(control_socket_from_yaml(file_content).unwrap(), None);
    }

    #[test]
    fn it_rejects_non_string_control_socket() {
        let file_content = r#"
on:
  socket: 42
tasks:
  - name: my tests
    run: cargo test
    run_on_init: true
"#;

        assert!(control_socket_from_yaml(file_content)
            .unwrap_err()
            .contains("on.socket"));
    }

    #[test]
    fn test_yaml_loader_returns_empty_for_invalid_content() {
        let file_content = "
        - name: this is valid
          run: 'cargo tests'
          change: '**/*'

        - name: this is invalid
          run: 'cargo tests'
          change: **/*
        ";

        let content = YamlLoader::load_from_str(file_content);
        assert!(content.is_err());
    }

    #[test]
    fn it_loads_from_args() {
        let file_content = "
        - name: my test
          run: 'cargo tests'
          change: 'bla/**'
          ignore: 'foo/**'
        ";

        let content = YamlLoader::load_from_str(file_content).unwrap();
        let rule = rule_from(&content[0][0]).unwrap();

        let result = rule.commands();
        assert_eq!(vec!["cargo tests"], result);
    }

    fn get_absolute_path(path: &str) -> String {
        let mut absolute_path = current_dir().unwrap();
        absolute_path.push(path);
        absolute_path.to_str().unwrap().to_string()
    }

    #[test]
    fn it_does_not_filters_empty_or_one_character_path() {
        let content = "./foo\n./bar\n.\n./baz\n"
            .lines()
            .map(|s| s.to_owned())
            .collect();
        let rules = from_argv(content, vec!["cargo test".to_owned()]).unwrap();
        assert!(rules[0].watch(&get_absolute_path("foo")));
        assert!(rules[0].watch(&get_absolute_path("bar")));
        assert!(rules[0].watch(&get_absolute_path("baz")));
        assert!(rules[0].watch(&get_absolute_path(".")));
    }

    #[test]
    fn it_formats_rule_as_yaml_string() {
        let file_content = "
        - name: my tests
          run: cargo tests {{filepath}}
          change: 'tests/**'
          run_on_init: true

        - name: my tests
          run: ['echo {{filepath}}', 'make tests {{filepath}}']
          change: 'tests/**'
        ";

        let rules = from_yaml(file_content).expect("Failed to parse yaml");

        assert_eq!(
            rule_as_yaml(&rules[0]),
            [
                "name: my tests",
                "run: cargo tests {{filepath}}",
                "change: tests/**",
                "run_on_init: true"
            ]
            .join("\n"),
            "Failed to format rule as string {}",
            rule_as_yaml(&rules[0])
        );
    }

    #[test]
    fn it_formats_manual_rule_as_yaml_with_trigger() {
        // MANUAL-TRIGGER-CONTRACT §6 + QA P2: verbose render must keep
        // `trigger: manual` — a rendered manual job has no change/init
        // surface, so dropping the property yields an invalid config.
        let file_content = "
        jobs:
          - name: await-remote
            run: ./scripts/await-remote.sh
            trigger: manual
        ";

        let rules = from_yaml(file_content).expect("Failed to parse yaml");

        let rendered = rule_as_yaml(&rules[0]);
        assert_eq!(
            rendered,
            [
                "name: await-remote",
                "run: ./scripts/await-remote.sh",
                "trigger: manual",
            ]
            .join("\n"),
            "Failed to format manual rule as string {}",
            rendered
        );

        // Round-trip: the rendered form parses back and validates (the P2
        // failure mode — silent trigger loss — must be impossible).
        let round_tripped = from_yaml(&format!(
            "jobs:\n{}\n",
            rendered
                .replace("name: ", "  - name: ")
                .replace("\nrun: ", "\n    run: ")
                .replace("\ntrigger: ", "\n    trigger: ")
        ))
        .expect("rendered config must re-parse");
        assert!(round_tripped[0].validate().is_ok());
        assert_eq!(
            round_tripped[0].trigger(),
            Some(crate::rules::TriggerMode::Manual)
        );
    }

    #[test]
    fn it_formats_timeout_rule_as_yaml_with_canonical_duration() {
        // FINITE-JOB-TIMEOUT-CONTRACT §1 + QA P2: verbose render must keep
        // `timeout` — dropping it would silently unbound a bounded job.
        // Canonical rendering: largest unit that divides evenly (m > s > ms).
        let file_content = "
        jobs:
          - name: await-remote
            run: ./scripts/await-remote.sh
            trigger: manual
            timeout: 90s
        ";

        let rules = from_yaml(file_content).expect("Failed to parse yaml");

        let rendered = rule_as_yaml(&rules[0]);
        assert_eq!(
            rendered,
            [
                "name: await-remote",
                "run: ./scripts/await-remote.sh",
                "trigger: manual",
                "timeout: 90s",
            ]
            .join("\n"),
            "Failed to format timeout rule as string {}",
            rendered
        );

        // Round-trip: the rendered form re-parses to the same deadline.
        let round_tripped = from_yaml(&format!(
            "jobs:\n{}\n",
            rendered
                .replace("name: ", "  - name: ")
                .replace("\nrun: ", "\n    run: ")
                .replace("\ntrigger: ", "\n    trigger: ")
                .replace("\ntimeout: ", "\n    timeout: ")
        ))
        .expect("rendered config must re-parse");
        assert_eq!(
            round_tripped[0].timeout(),
            rules[0].timeout(),
            "rendered timeout must round-trip to the same deadline"
        );
    }

    #[test]
    fn it_fails_for_invalid_watch_file_format() {
        let file_content = "
        - name: this is valid
          run: 'cargo tests'
          change: '**/*'

        - name: this is invalid
          run: 'cargo tests'
          change: **/*
        ";

        let result = from_yaml(file_content);
        assert!(result.is_err());
        assert_eq!(
            result.err().unwrap().to_string(),
            ["Failed to load configuration at line:",
                "|           run: 'cargo tests'",
                "|>          change: **/*",
                "|         ",
                "Reason: while parsing node, found unknown anchor at byte 165 line 8 column 19",
                "Hint: Check for wrong types, any missing quotes for glob pattern or incorrect identation"]
            .join("\n")
        );

        let empty_file = "
        ";

        let result = from_yaml(empty_file);
        assert!(result.is_err());
        assert_eq!(
            result.err().unwrap().to_string(),
            ["Configuration file is invalid! There are no rules to watch",
                "Hint: Make sure to declare at least one rule. Try to run `fzz init` to generate a new configuration from scratch"]
            .join("\n")
        );

        let invalid_hash_file = "
        on:
            - name: foo
              run: echo foo
        ";

        let result = from_yaml(invalid_hash_file);
        assert!(result.is_err());
        assert_eq!(
            result.err().unwrap().to_string(),
            ["Configuration file is invalid. When using the 'on' format, you must provide a 'tasks' array",
                "Hint: Example:",
                "on:",
                "  change: [\"src/**\"]",
                "jobs:",
                "  - name: build",
                "    run: cargo build"]
            .join("\n")
        );
    }

    #[test]
    fn it_validates_missing_properties() {
        let rules_yaml = from_yaml(
            "
        - name: rules must have at least one command
          change:
            - '**/*.go'

        - name: missing trigger property
          run: 'echo invalid'
          ignore: '**/*.go'
        ",
        );
        assert!(rules_yaml.is_err());
    }

    #[test]
    fn it_rejects_legacy_lua_entries_in_change() {
        let file_content = "
        - name: lua task
          run: 'echo lua'
          change: ':lua onchange.lua'
        ";

        let content = YamlLoader::load_from_str(file_content).unwrap();
        let err = rule_from(&content[0][0]).expect_err("Expected :lua entries to be rejected");
        let message = format!("{}", err);
        assert!(
            message.contains("Property 'change' no longer accepts ':lua' entries."),
            "Unexpected error: {}",
            message
        );
    }

    #[test]
    fn it_rejects_legacy_lua_entries_in_ignore() {
        let file_content = "
        - name: lua task
          run: 'echo lua'
          change: '**/*.txt'
          ignore: ':lua ignore.lua'
        ";

        let content = YamlLoader::load_from_str(file_content).unwrap();
        let err = rule_from(&content[0][0]).expect_err("Expected :lua entries to be rejected");
        let message = format!("{}", err);
        assert!(
            message.contains("Property 'ignore' no longer accepts ':lua' entries."),
            "Unexpected error: {}",
            message
        );
    }

    // Tests for common rules format (on + tasks)

    #[test]
    fn it_parses_common_rules_with_on_and_tasks() {
        let file_content = "
on:
  change:
    - 'src/**'
    - 'tests/**'
  ignore:
    - '**/*.log'

tasks:
  - name: build
    run: 'cargo build'

  - name: test
    run: 'cargo test'
        ";

        let rules = from_yaml(file_content).expect("Failed to parse yaml");
        assert_eq!(rules.len(), 2);

        // Both tasks should inherit the common change and ignore patterns
        assert_eq!(rules[0].name, "build");
        assert_eq!(rules[0].watch_patterns(), vec!["src/**", "tests/**"]);
        assert_eq!(rules[0].ignore_glob_patterns(), vec!["**/*.log"]);

        assert_eq!(rules[1].name, "test");
        assert_eq!(rules[1].watch_patterns(), vec!["src/**", "tests/**"]);
        assert_eq!(rules[1].ignore_glob_patterns(), vec!["**/*.log"]);
    }

    #[test]
    fn it_merges_common_change_with_task_change() {
        let file_content = "
on:
  change:
    - 'src/**'
  ignore:
    - '**/*.log'

tasks:
  - name: build
    run: 'cargo build'

  - name: test
    run: 'cargo test'
    change: 'tests/**'
        ";

        let rules = from_yaml(file_content).expect("Failed to parse yaml");
        assert_eq!(rules.len(), 2);

        // First task has only the common patterns
        assert_eq!(rules[0].watch_patterns(), vec!["src/**"]);
        assert_eq!(rules[0].ignore_glob_patterns(), vec!["**/*.log"]);

        // Second task EXTENDS common change; root scope always applies
        assert_eq!(rules[1].watch_patterns(), vec!["src/**", "tests/**"]);
        assert_eq!(rules[1].ignore_glob_patterns(), vec!["**/*.log"]);
    }

    #[test]
    fn it_merges_common_ignore_with_task_ignore() {
        let file_content = "
on:
  change:
    - 'src/**'
  ignore:
    - '**/*.log'

tasks:
  - name: build
    run: 'cargo build'
    ignore:
      - '**/*.tmp'
      - 'target/**'
        ";

        let rules = from_yaml(file_content).expect("Failed to parse yaml");
        assert_eq!(rules.len(), 1);

        // Task extends common ignore; root safety rails always apply
        assert_eq!(rules[0].watch_patterns(), vec!["src/**"]);
        assert_eq!(
            rules[0].ignore_glob_patterns(),
            vec!["**/*.log", "**/*.tmp", "target/**"]
        );
    }

    #[test]
    fn it_dedupes_common_and_task_patterns() {
        let file_content = "
on:
  change:
    - 'src/**'
  ignore:
    - '**/*.log'

tasks:
  - name: build
    run: 'cargo build'
    change:
      - 'src/**'
      - 'tests/**'
    ignore:
      - '**/*.log'
      - '**/*.tmp'
        ";

        let rules = from_yaml(file_content).expect("Failed to parse yaml");
        assert_eq!(rules.len(), 1);
        assert_eq!(rules[0].watch_patterns(), vec!["src/**", "tests/**"]);
        assert_eq!(
            rules[0].ignore_glob_patterns(),
            vec!["**/*.log", "**/*.tmp"]
        );
    }

    #[test]
    fn it_allows_on_without_change() {
        let file_content = "
on:
  ignore:
    - '**/*.log'

tasks:
  - name: build
    run: 'cargo build'
    change: 'src/**'
        ";

        let rules = from_yaml(file_content).expect("Failed to parse yaml");
        assert_eq!(rules.len(), 1);
        assert_eq!(rules[0].watch_patterns(), vec!["src/**"]);
        assert_eq!(rules[0].ignore_glob_patterns(), vec!["**/*.log"]);
    }

    #[test]
    fn it_allows_on_without_ignore() {
        let file_content = "
on:
  change:
    - 'src/**'

tasks:
  - name: build
    run: 'cargo build'
        ";

        let rules = from_yaml(file_content).expect("Failed to parse yaml");
        assert_eq!(rules.len(), 1);
        assert_eq!(rules[0].watch_patterns(), vec!["src/**"]);
        assert_eq!(rules[0].ignore_glob_patterns().len(), 0);
    }

    #[test]
    fn it_allows_empty_on_section() {
        let file_content = "
on: {}

tasks:
  - name: build
    run: 'cargo build'
    change: 'src/**'
        ";

        let rules = from_yaml(file_content).expect("Failed to parse yaml");
        assert_eq!(rules.len(), 1);
        assert_eq!(rules[0].watch_patterns(), vec!["src/**"]);
    }

    #[test]
    fn it_allows_missing_on_section() {
        let file_content = "
tasks:
  - name: build
    run: 'cargo build'
    change: 'src/**'
        ";

        let rules = from_yaml(file_content).expect("Failed to parse yaml");
        assert_eq!(rules.len(), 1);
        assert_eq!(rules[0].watch_patterns(), vec!["src/**"]);
    }

    #[test]
    fn it_fails_when_tasks_is_missing() {
        let file_content = "
on:
  change:
    - 'src/**'
        ";

        let result = from_yaml(file_content);
        assert!(result.is_err());
        let err = result.err().unwrap().to_string();
        assert!(err.contains("must provide a 'tasks' array"));
    }

    #[test]
    fn it_fails_when_tasks_is_not_array() {
        let file_content = "
on:
  change:
    - 'src/**'
tasks:
  name: build
  run: cargo build
        ";

        let result = from_yaml(file_content);
        assert!(result.is_err());
        let err = result.err().unwrap().to_string();
        assert!(
            err.contains("'tasks' must be an Array/List")
                || err.contains("'tasks' must be an ordered list")
        );
    }

    #[test]
    fn it_fails_when_on_has_invalid_properties() {
        let file_content = "
on:
  change:
    - 'src/**'
  invalid_prop: foo

tasks:
  - name: build
    run: cargo build
        ";

        let result = from_yaml(file_content);
        assert!(result.is_err());
        let err = result.err().unwrap().to_string();
        assert!(err.contains("Invalid property 'on.invalid_prop'"));
    }

    #[test]
    fn it_validates_glob_patterns_in_common_rules() {
        let file_content = "
on:
  change:
    - '**/foo_**.go'

tasks:
  - name: build
    run: cargo build
        ";

        let rules = from_yaml(file_content);
        assert!(rules.is_ok());

        let rules = rules.unwrap();
        let validation = rules[0].validate();
        assert!(validation.is_err());
        assert!(validation
            .err()
            .unwrap()
            .contains("invalid `change` glob pattern"));
    }

    #[test]
    fn it_supports_run_on_init_with_common_rules() {
        let file_content = "
on:
  change:
    - 'src/**'

tasks:
  - name: init_task
    run: 'echo init'
    run_on_init: true
        ";

        let rules = from_yaml(file_content).expect("Failed to parse yaml");
        assert_eq!(rules.len(), 1);
        assert!(rules[0].run_on_init());
        assert_eq!(rules[0].watch_patterns(), vec!["src/**"]);
    }

    #[test]
    fn it_allows_task_with_only_run_on_init_no_change() {
        let file_content = "
tasks:
  - name: init_only
    run: 'echo startup'
    run_on_init: true
        ";

        let rules = from_yaml(file_content).expect("Failed to parse yaml");
        assert_eq!(rules.len(), 1);
        assert!(rules[0].run_on_init());
        assert_eq!(rules[0].watch_patterns().len(), 0);
        assert!(rules[0].validate().is_ok());
    }

    #[test]
    fn it_parses_parallel_group_name_on_task() {
        let file_content = "
        - name: lint
          parallel: checks
          run: make lint
          change: 'src/**'

        - name: test
          parallel: checks
          run: make test
          change: 'src/**'
        ";

        let rules = from_yaml(file_content).expect("Failed to parse yaml");
        assert_eq!(rules.len(), 2);
        assert_eq!(rules[0].parallel(), Some("checks"));
        assert_eq!(rules[1].parallel(), Some("checks"));
    }

    #[test]
    fn it_parses_parallel_group_in_hash_format_with_common_rules() {
        let file_content = "
        on:
          change: 'src/**'
        execution:
          concurrency: 2
        tasks:
          - name: lint
            parallel: checks
            cwd: crates/lint
            env: { MODE: strict }
            run: make lint
          - name: test
            run: make test
        ";

        let rules = from_yaml(file_content).expect("Failed to parse yaml");
        assert_eq!(rules[0].parallel(), Some("checks"));
        assert_eq!(rules[0].cwd(), Some("crates/lint"));
        assert_eq!(
            rules[0].environment().get("MODE").map(String::as_str),
            Some("strict")
        );
        assert_eq!(rules[1].parallel(), None, "no parallel means serial");
    }

    #[test]
    fn it_parses_optional_task_cwd_and_string_environment() {
        let file_content = "
        - name: web
          cwd: packages/web app
          env:
            NODE_ENV: test
            EMPTY: ''
          run: npm test
          change: 'packages/**'
        ";

        let rules = from_yaml(file_content).expect("task context must parse");
        assert_eq!(rules[0].cwd(), Some("packages/web app"));
        assert_eq!(
            rules[0].environment().get("NODE_ENV").map(String::as_str),
            Some("test")
        );
        assert_eq!(
            rules[0].environment().get("EMPTY").map(String::as_str),
            Some("")
        );
    }

    #[test]
    fn legacy_task_defaults_to_workspace_cwd_and_inherited_environment() {
        let rules =
            from_yaml("- name: test\n  run: cargo test\n  change: src/**\n").expect("legacy task");
        assert_eq!(rules[0].cwd(), None);
        assert!(rules[0].environment().is_empty());
    }

    #[test]
    fn it_rejects_wrong_task_context_types_and_empty_environment_names() {
        let wrong_cwd =
            from_yaml("- name: test\n  cwd: true\n  run: cargo test\n  change: src/**\n")
                .expect_err("cwd must be a string");
        assert!(format!("{}", wrong_cwd).contains("'cwd'"));

        let wrong_env =
            from_yaml("- name: test\n  env: [A]\n  run: cargo test\n  change: src/**\n")
                .expect_err("env must be a map");
        assert!(format!("{}", wrong_env).contains("Property 'env'"));

        let non_string_value =
            from_yaml("- name: test\n  env: { A: 1 }\n  run: cargo test\n  change: src/**\n")
                .expect_err("env values must be strings");
        assert!(
            format!("{}", non_string_value).contains("Environment value for 'A' must be a string")
        );

        let empty_name =
            from_yaml("- name: test\n  env: { '': value }\n  run: cargo test\n  change: src/**\n")
                .expect_err("env names must be non-empty");
        assert!(format!("{}", empty_name).contains("Environment variable name cannot be empty"));
    }

    #[test]
    fn it_rejects_boolean_parallel_value() {
        let file_content = "
        - name: lint
          parallel: true
          run: make lint
          change: 'src/**'
        ";

        let err = from_yaml(file_content).expect_err("boolean parallel must fail");
        assert!(
            format!("{}", err).contains("Expected 'String' but got: Boolean"),
            "unexpected error: {}",
            err
        );
    }

    #[test]
    fn it_rejects_collection_parallel_value() {
        let file_content = "
        - name: lint
          parallel: [checks]
          run: make lint
          change: 'src/**'
        ";

        let err = from_yaml(file_content).expect_err("collection parallel must fail");
        assert!(
            format!("{}", err).contains("Expected 'String' but got: Array"),
            "unexpected error: {}",
            err
        );
    }

    #[test]
    fn it_rejects_empty_parallel_group_name() {
        let file_content = "
        - name: lint
          parallel: ''
          run: make lint
          change: 'src/**'
        ";

        let err = from_yaml(file_content).expect_err("empty parallel must fail");
        assert!(
            format!("{}", err).contains("Property 'parallel' cannot be empty"),
            "unexpected error: {}",
            err
        );
    }

    #[test]
    fn it_accepts_concurrency_from_execution_section() {
        let file_content = "
on:\n  change: 'src/**'\nexecution:\n  concurrency: 4\ntasks:\n  - name: lint\n    run: make lint\n";
        assert_eq!(concurrency_from_yaml(file_content).unwrap(), Some(4));
    }

    #[test]
    fn it_rejects_jobs_so_name_remains_available_for_workflow_items() {
        let file_content = "\non:\n  jobs: 4\ntasks:\n  - name: lint\n    run: make lint\n";
        let error = from_yaml(file_content).expect_err("on.jobs is not a supported alias");
        assert!(
            error.to_string().contains("Invalid property 'on.jobs'"),
            "unexpected: {}",
            error
        );
    }

    #[test]
    fn it_defaults_concurrency_when_absent() {
        let file_content = "
        - name: lint
          run: make lint
          change: 'src/**'
        ";
        assert_eq!(concurrency_from_yaml(file_content).unwrap(), None);
    }

    #[test]
    fn it_rejects_zero_and_negative_concurrency() {
        let zero = "\nexecution:\n  concurrency: 0\ntasks:\n  - name: a\n    run: echo a\n";
        let negative = "\nexecution:\n  concurrency: -2\ntasks:\n  - name: a\n    run: echo a\n";

        let zero_err = concurrency_from_yaml(zero).expect_err("zero concurrency must fail");
        assert!(
            zero_err.contains("positive integer"),
            "unexpected: {}",
            zero_err
        );
        let negative_err =
            concurrency_from_yaml(negative).expect_err("negative concurrency must fail");
        assert!(
            negative_err.contains("positive integer"),
            "unexpected: {}",
            negative_err
        );
    }

    #[test]
    fn it_rejects_non_integer_concurrency() {
        let file_content =
            "\nexecution:\n  concurrency: many\ntasks:\n  - name: a\n    run: echo a\n";
        let err = concurrency_from_yaml(file_content).expect_err("string concurrency must fail");
        assert!(err.contains("positive integer"), "unexpected: {}", err);
    }

    #[test]
    fn it_rejects_concurrency_outside_execution_object() {
        let file_content = "\nexecution: 3\ntasks:\n  - name: a\n    run: echo a\n";
        let err = concurrency_from_yaml(file_content).expect_err("scalar execution must fail");
        assert!(
            err.contains("'execution' must be an object"),
            "unexpected: {}",
            err
        );
    }

    #[test]
    fn it_watches_paths_correctly_with_common_rules() {
        let file_content = "
on:
  change: 'src/**'
  ignore: 'src/test/**'

tasks:
  - name: build
    run: 'cargo build'
        ";

        let rules = from_yaml(file_content).expect("Failed to parse yaml");
        assert_eq!(rules.len(), 1);

        // Should watch src files
        assert!(rules[0].watch("src/main.rs"));
        assert!(rules[0].watch("src/lib.rs"));

        // Should ignore test files
        assert!(rules[0].ignore("src/test/foo.rs"));
    }

    #[test]
    fn it_maintains_backward_compatibility_with_array_format() {
        let old_format = "
        - name: build
          run: 'cargo build'
          change: 'src/**'
          ignore: '**/*.log'

        - name: test
          run: 'cargo test'
          change: 'tests/**'
        ";

        let rules = from_yaml(old_format).expect("Failed to parse yaml");
        assert_eq!(rules.len(), 2);
        assert_eq!(rules[0].name, "build");
        assert_eq!(rules[1].name, "test");
    }

    #[test]
    fn it_parses_multiple_nested_groups_with_different_common_rules() {
        let file_content = "
        - on:
            change:
              - 'src/frontend/**'
              - 'public/**'
            ignore:
              - '**/*.log'
          tasks:
            - name: frontend-build
              run: npm run build
            - name: frontend-test
              run: npm test

        - on:
            change:
              - 'src/backend/**'
              - 'api/**'
            ignore:
              - 'target/**'
          tasks:
            - name: backend-build
              run: cargo build
            - name: backend-test
              run: cargo test
        ";

        let rules = from_yaml(file_content).expect("Failed to parse yaml");

        // Should have 4 rules total (2 from each group)
        assert_eq!(rules.len(), 4);

        // Frontend tasks should have frontend patterns
        assert_eq!(rules[0].name, "frontend-build");
        assert!(rules[0]
            .watch_patterns()
            .contains(&"src/frontend/**".to_string()));
        assert!(rules[0].watch_patterns().contains(&"public/**".to_string()));
        assert!(rules[0]
            .ignore_glob_patterns()
            .contains(&"**/*.log".to_string()));

        assert_eq!(rules[1].name, "frontend-test");
        assert!(rules[1]
            .watch_patterns()
            .contains(&"src/frontend/**".to_string()));

        // Backend tasks should have backend patterns
        assert_eq!(rules[2].name, "backend-build");
        assert!(rules[2]
            .watch_patterns()
            .contains(&"src/backend/**".to_string()));
        assert!(rules[2].watch_patterns().contains(&"api/**".to_string()));
        assert!(rules[2]
            .ignore_glob_patterns()
            .contains(&"target/**".to_string()));

        assert_eq!(rules[3].name, "backend-test");
        assert!(rules[3]
            .watch_patterns()
            .contains(&"src/backend/**".to_string()));
    }

    #[test]
    fn it_mixes_regular_tasks_and_nested_groups_in_same_array() {
        let file_content = "
        - name: regular-task
          run: echo 'regular'
          change: 'regular/**'

        - on:
            change: 'grouped/**'
          tasks:
            - name: group-task-1
              run: echo 'group1'
            - name: group-task-2
              run: echo 'group2'

        - name: another-regular
          run: echo 'another'
          change: 'another/**'
        ";

        let rules = from_yaml(file_content).expect("Failed to parse yaml");

        // Should have 4 rules: 1 regular + 2 grouped + 1 regular
        assert_eq!(rules.len(), 4);

        assert_eq!(rules[0].name, "regular-task");
        assert!(rules[0]
            .watch_patterns()
            .contains(&"regular/**".to_string()));

        assert_eq!(rules[1].name, "group-task-1");
        assert!(rules[1]
            .watch_patterns()
            .contains(&"grouped/**".to_string()));

        assert_eq!(rules[2].name, "group-task-2");
        assert!(rules[2]
            .watch_patterns()
            .contains(&"grouped/**".to_string()));

        assert_eq!(rules[3].name, "another-regular");
        assert!(rules[3]
            .watch_patterns()
            .contains(&"another/**".to_string()));
    }

    #[test]
    fn it_merges_common_rules_in_nested_group() {
        let file_content = "
        - on:
            change: 'src/**'
            ignore: '**/*.log'
          tasks:
            - name: inherits-all
              run: echo 'inherit'

            - name: extends-change
              run: echo 'extend'
              change: 'custom/**'

            - name: extends-ignore
              run: echo 'extend'
              ignore: '**/*.tmp'
        ";

        let rules = from_yaml(file_content).expect("Failed to parse yaml");
        assert_eq!(rules.len(), 3);

        // First task has only the common rules
        assert_eq!(rules[0].name, "inherits-all");
        assert!(rules[0].watch_patterns().contains(&"src/**".to_string()));
        assert!(rules[0]
            .ignore_glob_patterns()
            .contains(&"**/*.log".to_string()));

        // Second task extends change; common scope always applies
        assert_eq!(rules[1].name, "extends-change");
        assert!(rules[1].watch_patterns().contains(&"custom/**".to_string()));
        assert!(rules[1].watch_patterns().contains(&"src/**".to_string()));
        assert!(rules[1]
            .ignore_glob_patterns()
            .contains(&"**/*.log".to_string()));

        // Third task extends ignore; common safety rails always apply
        assert_eq!(rules[2].name, "extends-ignore");
        assert!(rules[2].watch_patterns().contains(&"src/**".to_string()));
        assert!(rules[2]
            .ignore_glob_patterns()
            .contains(&"**/*.tmp".to_string()));
        assert!(rules[2]
            .ignore_glob_patterns()
            .contains(&"**/*.log".to_string()));
    }

    #[test]
    fn it_parses_empty_tasks_array_in_nested_group() {
        let file_content = "
        - on:
            change: 'src/**'
          tasks: []

        - name: regular-task
          run: echo 'regular'
          change: 'other/**'
        ";

        let rules = from_yaml(file_content).expect("Failed to parse yaml");

        // Should only have 1 rule (the regular task)
        assert_eq!(rules.len(), 1);
        assert_eq!(rules[0].name, "regular-task");
    }

    #[test]
    fn it_handles_multiple_groups_watching_same_files() {
        let file_content = "
        - on:
            change: 'src/**'
          tasks:
            - name: group1-task
              run: echo 'group1'

        - on:
            change: 'src/**'
          tasks:
            - name: group2-task
              run: echo 'group2'
        ";

        let rules = from_yaml(file_content).expect("Failed to parse yaml");

        // Both groups should coexist
        assert_eq!(rules.len(), 2);
        assert_eq!(rules[0].name, "group1-task");
        assert_eq!(rules[1].name, "group2-task");

        // Both should watch the same pattern
        assert!(rules[0].watch_patterns().contains(&"src/**".to_string()));
        assert!(rules[1].watch_patterns().contains(&"src/**".to_string()));
    }

    #[test]
    fn it_maintains_backward_compatibility_with_all_formats() {
        // Test 1: Classic array format
        let classic = "
        - name: task1
          run: echo 'classic'
          change: 'src/**'
        ";
        let rules1 = from_yaml(classic).expect("Failed to parse classic format");
        assert_eq!(rules1.len(), 1);
        assert_eq!(rules1[0].name, "task1");

        // Test 2: Single group format
        let single_group = "
        on:
          change: 'src/**'
        tasks:
          - name: task2
            run: echo 'single'
        ";
        let rules2 = from_yaml(single_group).expect("Failed to parse single group format");
        assert_eq!(rules2.len(), 1);
        assert_eq!(rules2[0].name, "task2");

        // Test 3: Nested groups format
        let nested = "
        - on:
            change: 'src/**'
          tasks:
            - name: task3
              run: echo 'nested'
        ";
        let rules3 = from_yaml(nested).expect("Failed to parse nested groups format");
        assert_eq!(rules3.len(), 1);
        assert_eq!(rules3[0].name, "task3");
    }
}

#[cfg(test)]
mod debounce_tests {
    use super::*;
    use std::time::Duration;

    #[test]
    fn debounce_defaults_to_one_second_when_absent() {
        assert_eq!(debounce_from_yaml("on:\n  change: '**/*'\n").unwrap(), None);
        assert_eq!(debounce_from_yaml("tasks: []\n").unwrap(), None);
    }

    #[test]
    fn debounce_accepts_documented_duration_syntax() {
        assert_eq!(
            debounce_from_yaml("on:\n  debounce: 500ms\n").unwrap(),
            Some(Duration::from_millis(500))
        );
        assert_eq!(
            debounce_from_yaml("on:\n  debounce: 2s\n").unwrap(),
            Some(Duration::from_secs(2))
        );
        assert_eq!(
            debounce_from_yaml("on:\n  debounce: 3\n").unwrap(),
            Some(Duration::from_secs(3))
        );
        assert_eq!(
            debounce_from_yaml("on:\n  debounce: 1m\n").unwrap(),
            Some(Duration::from_secs(60))
        );
    }

    #[test]
    fn debounce_rejects_zero_and_invalid_values() {
        assert!(debounce_from_yaml("on:\n  debounce: 0\n").is_err());
        assert!(debounce_from_yaml("on:\n  debounce: 0ms\n").is_err());
        assert!(debounce_from_yaml("on:\n  debounce: -1\n").is_err());
        assert!(debounce_from_yaml("on:\n  debounce: fast\n").is_err());
        assert!(debounce_from_yaml("on:\n  debounce: 1h\n").is_err());
    }
}

#[cfg(test)]
mod jobs_tests {
    use super::*;

    #[test]
    fn jobs_root_parses_to_the_same_rules_as_tasks() {
        let jobs = from_yaml(
            "on:\n  change: '**/*'\njobs:\n  - name: lint\n    run: cargo clippy\n  - name: test\n    run: cargo test\n",
        )
        .expect("jobs parse");
        let tasks = from_yaml(
            "on:\n  change: '**/*'\ntasks:\n  - name: lint\n    run: cargo clippy\n  - name: test\n    run: cargo test\n",
        )
        .expect("tasks parse");
        assert_eq!(jobs.len(), 2);
        assert_eq!(jobs[0].name, "lint");
        assert_eq!(jobs[0].commands(), tasks[0].commands());
        assert_eq!(jobs[1].name, "test");
        assert_eq!(jobs[1].commands(), tasks[1].commands());
    }

    #[test]
    fn jobs_preserves_declaration_order_and_parallel_groups() {
        let rules = from_yaml(
            "on:\n  change: '**/*'\njobs:\n  - name: a\n    parallel: checks\n    run: echo a\n  - name: b\n    parallel: checks\n    run: echo b\n  - name: c\n    run: echo c\n",
        )
        .expect("jobs parse");
        let names: Vec<&str> = rules.iter().map(|rule| rule.name.as_str()).collect();
        assert_eq!(names, vec!["a", "b", "c"]);
        assert_eq!(
            rules[0].parallel().map(str::to_string),
            Some("checks".to_string())
        );
        assert_eq!(
            rules[1].parallel().map(str::to_string),
            Some("checks".to_string())
        );
        assert_eq!(rules[2].parallel(), None);
    }

    #[test]
    fn mixed_tasks_and_jobs_is_an_error() {
        let err = from_yaml(
            "on:\n  change: '**/*'\ntasks:\n  - name: a\n    run: echo a\njobs:\n  - name: b\n    run: echo b\n",
        )
        .expect_err("mixed keys must fail");
        assert!(format!("{:?}", err).contains("tasks"), "{err:?}");
        assert!(format!("{:?}", err).contains("jobs"), "{err:?}");
    }

    #[test]
    fn mapping_form_jobs_is_rejected_with_ordered_list_hint() {
        let err = from_yaml("on:\n  change: '**/*'\njobs:\n  lint: { run: cargo clippy }\n")
            .expect_err("mapping form must fail");
        let message = format!("{:?}", err);
        assert!(message.contains("jobs"), "{message}");
    }

    #[test]
    fn empty_jobs_is_an_error() {
        assert!(from_yaml("on:\n  change: '**/*'\njobs: []\n").is_err());
        assert!(from_yaml("on:\n  change: '**/*'\njobs:\n").is_err());
    }

    #[test]
    fn scalar_and_null_job_entries_are_rejected() {
        assert!(from_yaml("on:\n  change: '**/*'\njobs:\n  - hello\n").is_err());
        assert!(from_yaml("on:\n  change: '**/*'\njobs:\n  - null\n").is_err());
    }

    #[test]
    fn duplicate_job_names_are_rejected() {
        assert!(
            from_yaml(
                "on:\n  change: '**/*'\njobs:\n  - name: dup\n    run: echo a\n  - name: dup\n    run: echo b\n"
            )
            .is_err()
        );
    }

    #[test]
    fn recovery_accepts_scalar_and_preserves_ordered_list() {
        let scalar = from_yaml("jobs:\n  - name: format\n    run: check\n    recovery: format\n")
            .expect("scalar recovery");
        assert_eq!(
            scalar[0].recovery_commands(),
            Some(vec!["format".to_owned()])
        );

        let list = from_yaml(
            "jobs:\n  - name: format\n    run: check\n    recovery:\n      - format\n      - git diff --check\n",
        )
        .expect("list recovery");
        assert_eq!(
            list[0].recovery_commands(),
            Some(vec!["format".to_owned(), "git diff --check".to_owned()])
        );
    }

    #[test]
    fn recovery_is_absent_or_rejected_without_silent_coercion() {
        let absent = from_yaml("jobs:\n  - name: check\n    run: check\n").unwrap();
        assert_eq!(absent[0].recovery_commands(), None);

        for recovery in ["''", "[]", "{}", "true", "[format, true]"] {
            let yaml =
                format!("jobs:\n  - name: check\n    run: check\n    recovery: {recovery}\n");
            assert!(from_yaml(&yaml).is_err(), "recovery must reject {recovery}");
        }
    }

    #[test]
    fn recovery_rejects_service_jobs_and_legacy_shapes() {
        assert!(from_yaml(
            "jobs:\n  - name: server\n    run: run\n    service: true\n    recovery: restart\n"
        )
        .is_err());
        assert!(from_yaml("- name: check\n  run: check\n  recovery: repair\n").is_err());
        assert!(
            from_yaml("tasks:\n  - name: check\n    run: check\n    recovery: repair\n").is_err()
        );
    }
}

#[cfg(test)]
mod backend_tests {
    use super::*;

    #[test]
    fn watch_backend_defaults_to_auto() {
        assert_eq!(
            watch_backend_from_yaml("on:\n  change: '**/*'\n").unwrap(),
            None
        );
        assert_eq!(watch_backend_from_yaml("tasks: []\n").unwrap(), None);
    }

    #[test]
    fn watch_backend_accepts_native_poll_and_auto() {
        assert_eq!(
            watch_backend_from_yaml("on:\n  watch_backend: native\n").unwrap(),
            Some(crate::watcher::WatchBackend::Native)
        );
        assert_eq!(
            watch_backend_from_yaml("on:\n  watch_backend: auto\n").unwrap(),
            Some(crate::watcher::WatchBackend::Auto)
        );
        assert_eq!(
            watch_backend_from_yaml("on:\n  watch_backend: poll\n  poll_interval: 200ms\n")
                .unwrap(),
            Some(crate::watcher::WatchBackend::Poll {
                interval: Duration::from_millis(200)
            })
        );
    }

    #[test]
    fn watch_backend_rejects_invalid_values() {
        assert!(watch_backend_from_yaml("on:\n  watch_backend: bogus\n").is_err());
        assert!(
            watch_backend_from_yaml("on:\n  watch_backend: poll\n  poll_interval: 0\n").is_err()
        );
    }
}

/// Parses the optional `on.watch_backend` (native|poll|auto) plus
/// `on.poll_interval` duration. Absent defaults to auto (native first, poll
/// fallback). Zero/invalid values are rejected loudly.
pub fn watch_backend_from_yaml(
    content: &str,
) -> Result<Option<crate::watcher::WatchBackend>, String> {
    let documents = YamlLoader::load_from_str(content).map_err(|err| err.to_string())?;
    let root = documents
        .first()
        .ok_or_else(|| "Configuration file is empty".to_owned())?;
    let on = &root["on"];

    if on == &Yaml::BadValue {
        return Ok(None);
    }
    if !matches!(on, Yaml::Hash(_)) {
        return Err("Property 'on' must be an object".to_owned());
    }
    if on["watch_backend"] == Yaml::BadValue {
        return Ok(None);
    }

    let backend = match &on["watch_backend"] {
        Yaml::String(value) => value.clone(),
        _ => return Err("Property 'on.watch_backend' must be a string".to_owned()),
    };
    let poll_interval = match &on["poll_interval"] {
        Yaml::BadValue => None,
        Yaml::Integer(value) => Some(value.to_string()),
        Yaml::String(value) => Some(value.clone()),
        _ => return Err("Property 'on.poll_interval' must be a duration".to_owned()),
    };
    let poll_interval = match poll_interval {
        None => None,
        Some(raw) => parse_debounce(&raw)?.map(|d| d.max(Duration::from_millis(20))),
    };
    crate::watcher::WatchBackend::parse(Some(&backend), poll_interval).map(Some)
}

pub fn watch_backend_from_file(
    filename: &str,
) -> Result<Option<crate::watcher::WatchBackend>, String> {
    let mut file = File::open(filename).map_err(|err| err.to_string())?;
    let mut content = String::new();
    file.read_to_string(&mut content)
        .map_err(|err| err.to_string())?;
    watch_backend_from_yaml(&content)
}

#[cfg(test)]
mod gitignore_config_tests {
    use super::*;

    #[test]
    fn respect_gitignore_defaults_to_false() {
        assert!(!respect_gitignore_from_yaml("on:\n  change: '**/*'\n").unwrap());
    }

    #[test]
    fn respect_gitignore_parses_boolean() {
        assert!(respect_gitignore_from_yaml("on:\n  respect_gitignore: true\n").unwrap());
        assert!(!respect_gitignore_from_yaml("on:\n  respect_gitignore: false\n").unwrap());
    }

    #[test]
    fn respect_gitignore_rejects_non_boolean() {
        assert!(respect_gitignore_from_yaml("on:\n  respect_gitignore: yes-please\n").is_err());
    }
}

/// Parses the optional `on.respect_gitignore` boolean (default false).
pub fn respect_gitignore_from_yaml(content: &str) -> Result<bool, String> {
    let documents = YamlLoader::load_from_str(content).map_err(|err| err.to_string())?;
    let root = documents
        .first()
        .ok_or_else(|| "Configuration file is empty".to_owned())?;
    let on = &root["on"];
    if on == &Yaml::BadValue || on["respect_gitignore"] == Yaml::BadValue {
        return Ok(false);
    }
    match &on["respect_gitignore"] {
        Yaml::Boolean(value) => Ok(*value),
        _ => Err("Property 'on.respect_gitignore' must be a boolean".to_owned()),
    }
}

pub fn respect_gitignore_from_file(filename: &str) -> Result<bool, String> {
    let mut file = File::open(filename).map_err(|err| err.to_string())?;
    let mut content = String::new();
    file.read_to_string(&mut content)
        .map_err(|err| err.to_string())?;
    respect_gitignore_from_yaml(&content)
}

#[cfg(test)]
mod hooks_tests {
    use super::*;

    #[test]
    fn hooks_default_to_none() {
        let hooks = generation_hooks_from_yaml("hooks: {}\n").unwrap();
        assert_eq!(hooks.success, None);
        assert_eq!(hooks.failure, None);
    }

    #[test]
    fn hooks_parse_success_and_failure_commands() {
        let hooks = generation_hooks_from_yaml(
            "hooks:\n  success: 'echo done > done.txt'\n  failure: 'echo failed > failed.txt'\n",
        )
        .unwrap();
        assert_eq!(hooks.success.as_deref(), Some("echo done > done.txt"));
        assert_eq!(hooks.failure.as_deref(), Some("echo failed > failed.txt"));
    }

    #[test]
    fn legacy_grouped_tasks_keep_historical_on_hook_placement() {
        let yaml = "on:\n  success: echo ok\n  close: echo closed\ntasks:\n  - name: test\n    run: cargo test\n";
        assert_eq!(
            generation_hooks_from_yaml(yaml).unwrap().success.as_deref(),
            Some("echo ok")
        );
        assert_eq!(
            session_hooks_from_yaml(yaml).unwrap().close.as_deref(),
            Some("echo closed")
        );
    }

    #[test]
    fn hooks_reject_non_string_values() {
        assert!(generation_hooks_from_yaml("hooks:\n  success: [a, b]\n").is_err());
        assert!(generation_hooks_from_yaml("hooks:\n  failure: 1\n").is_err());
        let settled =
            generation_hooks_from_yaml("hooks:\n  failure:\n    run: notify\n    settle: 30s\n")
                .unwrap();
        assert_eq!(settled.failure.as_deref(), Some("notify"));
        assert_eq!(settled.failure_settle, Some(Duration::from_secs(30)));
        let boundary =
            generation_hooks_from_yaml("hooks:\n  failure:\n    run: notify\n    settle: 1440m\n")
                .unwrap();
        assert_eq!(
            boundary.failure_settle,
            Some(Duration::from_secs(24 * 60 * 60))
        );
        assert!(generation_hooks_from_yaml(
            "hooks:\n  failure:\n    run: notify\n    settle: 0s\n"
        )
        .is_err());
        assert!(generation_hooks_from_yaml(
            "hooks:\n  failure:\n    run: notify\n    settle: 1s\n    extra: nope\n"
        )
        .is_err());
        assert!(generation_hooks_from_yaml(
            "hooks:\n  failure:\n    run: notify\n    settle: 1441m\n"
        )
        .is_err());
    }

    #[test]
    fn session_hook_defaults_to_none_and_parses_close_command() {
        assert_eq!(
            session_hooks_from_yaml("hooks: {}\n").unwrap(),
            SessionHooks::default()
        );
        assert_eq!(
            session_hooks_from_yaml("hooks:\n  close: './scripts/cleanup'\n")
                .unwrap()
                .close
                .as_deref(),
            Some("./scripts/cleanup")
        );
    }

    #[test]
    fn session_hook_rejects_non_string_empty_and_trigger_templates() {
        for yaml in [
            "hooks:\n  close: [a, b]\n",
            "hooks:\n  close: ''\n",
            "hooks:\n  close: 'echo {{filepath}}'\n",
            "hooks:\n  close: 'echo {{paths}}'\n",
        ] {
            assert!(
                session_hooks_from_yaml(yaml).is_err(),
                "must reject: {yaml}"
            );
        }
    }
}

/// Generation terminal hooks (`on.success` / `on.failure`), TASK-0040.
/// Kept distinct from [`SessionHooks`] so finite runners cannot execute the
/// watcher lifecycle hook accidentally.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct GenerationHooks {
    // failure_settle defaults to None for legacy callers
    pub success: Option<String>,
    pub failure: Option<String>,
    /// Optional asynchronous settlement window for failure hooks.
    pub failure_settle: Option<Duration>,
}

/// Watcher-session lifecycle hook (`on.close`), TASK-0101.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct SessionHooks {
    pub close: Option<String>,
}

/// Parses `on.success` / `on.failure` hook commands; absent = None,
/// non-string values are rejected loudly.
const MAX_FAILURE_SETTLE: Duration = Duration::from_secs(24 * 60 * 60);

pub fn generation_hooks_from_yaml(content: &str) -> Result<GenerationHooks, String> {
    let documents = YamlLoader::load_from_str(content).map_err(|err| err.to_string())?;
    let root = documents
        .first()
        .ok_or_else(|| "Configuration file is empty".to_owned())?;
    let hooks = &root["hooks"];
    let hooks = if hooks == &Yaml::BadValue && root["tasks"] != Yaml::BadValue {
        &root["on"]
    } else {
        hooks
    };
    if hooks == &Yaml::BadValue {
        return Ok(GenerationHooks::default());
    }
    if !matches!(hooks, Yaml::Hash(_)) {
        return Err("Property 'hooks' must be an object".to_owned());
    }
    let read_hook = |key: &str| -> Result<Option<String>, String> {
        match &hooks[key] {
            Yaml::BadValue => Ok(None),
            Yaml::String(value) if value.trim().is_empty() => Err(format!(
                "Property 'hooks.{}' must be a non-empty command string",
                key
            )),
            Yaml::String(value) => Ok(Some(value.clone())),
            _ => Err(format!("Property 'hooks.{}' must be a command string", key)),
        }
    };
    let failure_value = &hooks["failure"];
    let (failure, failure_settle) = match failure_value {
        Yaml::Hash(map) => {
            let run = match map.get(&Yaml::String("run".into())) {
                Some(Yaml::String(value)) if !value.trim().is_empty() => value.clone(),
                _ => {
                    return Err(
                        "Property 'hooks.failure.run' must be a non-empty command string".into(),
                    )
                }
            };
            let settle = match map.get(&Yaml::String("settle".into())) {
                Some(Yaml::String(value)) => parse_duration("hooks.failure.settle", value)?,
                Some(Yaml::Integer(value)) => {
                    parse_duration("hooks.failure.settle", &format!("{value}s"))?
                }
                _ => {
                    return Err(
                        "Property 'hooks.failure.settle' must be a positive duration".into(),
                    )
                }
            };
            let settle = settle.ok_or_else(|| {
                "Property 'hooks.failure.settle' must be greater than zero".to_owned()
            })?;
            if settle.is_zero() {
                return Err("Property 'hooks.failure.settle' must be greater than zero".into());
            }
            if settle > MAX_FAILURE_SETTLE {
                return Err("Property 'hooks.failure.settle' must not exceed 24h".into());
            }
            for key in map.keys() {
                if let Yaml::String(key) = key {
                    if key != "run" && key != "settle" {
                        return Err(format!(
                            "Unknown property 'hooks.failure.{key}' (expected run or settle)"
                        ));
                    }
                }
            }
            (Some(run), Some(settle))
        }
        _ => (read_hook("failure")?, None),
    };
    Ok(GenerationHooks {
        // failure_settle defaults to None for legacy callers
        success: read_hook("success")?,
        failure,
        failure_settle,
    })
}

pub fn generation_hooks_from_file(filename: &str) -> Result<GenerationHooks, String> {
    let mut file = File::open(filename).map_err(|err| err.to_string())?;
    let mut content = String::new();
    file.read_to_string(&mut content)
        .map_err(|err| err.to_string())?;
    generation_hooks_from_yaml(&content)
}

/// Parses watcher-session hooks. `close` has no trigger path, so trigger-bound
/// templates are rejected at config validation instead of expanding to empty.
pub fn session_hooks_from_yaml(content: &str) -> Result<SessionHooks, String> {
    let documents = YamlLoader::load_from_str(content).map_err(|err| err.to_string())?;
    let root = documents
        .first()
        .ok_or_else(|| "Configuration file is empty".to_owned())?;
    let hooks = &root["hooks"];
    let hooks = if hooks == &Yaml::BadValue && root["tasks"] != Yaml::BadValue {
        &root["on"]
    } else {
        hooks
    };
    if hooks == &Yaml::BadValue {
        return Ok(SessionHooks::default());
    }
    if !matches!(hooks, Yaml::Hash(_)) {
        return Err("Property 'hooks' must be an object".to_owned());
    }
    let close = match &hooks["close"] {
        Yaml::BadValue => None,
        Yaml::String(value) if value.trim().is_empty() => {
            return Err("Property 'hooks.close' must be a non-empty command string".to_owned())
        }
        Yaml::String(value) => Some(value.clone()),
        _ => return Err("Property 'hooks.close' must be a command string".to_owned()),
    };
    if let Some(command) = &close {
        for template in [
            "{{filepath}}",
            "{{absolute_path}}",
            "{{relative_filepath}}",
            "{{relative_path}}",
            "{{paths}}",
        ] {
            if command.contains(template) {
                return Err(format!(
                    "Property 'hooks.close' cannot use {template}: close has no trigger path"
                ));
            }
        }
    }
    Ok(SessionHooks { close })
}

pub fn session_hooks_from_file(filename: &str) -> Result<SessionHooks, String> {
    let mut file = File::open(filename).map_err(|err| err.to_string())?;
    let mut content = String::new();
    file.read_to_string(&mut content)
        .map_err(|err| err.to_string())?;
    session_hooks_from_yaml(&content)
}

#[cfg(test)]
mod output_policy_tests {
    use super::*;

    #[test]
    fn output_policy_defaults_to_inherit() {
        assert_eq!(
            output_policy_from_yaml("on:\n  change: '**/*'\n").unwrap(),
            OutputPolicy::Inherit
        );
        assert_eq!(
            output_policy_from_yaml("jobs:\n  - name: a\n    run: echo a\n    change: '**/*'\n")
                .unwrap(),
            OutputPolicy::Inherit
        );
    }

    #[test]
    fn output_policy_parses_all_values() {
        for (raw, expected) in [
            ("inherit", OutputPolicy::Inherit),
            ("quiet", OutputPolicy::Quiet),
            ("capture", OutputPolicy::Capture),
            ("show-on-failure", OutputPolicy::ShowOnFailure),
        ] {
            let yaml = format!("execution:\n  output: {raw}\n");
            assert_eq!(output_policy_from_yaml(&yaml).unwrap(), expected, "{raw}");
        }
    }

    #[test]
    fn output_policy_rejects_unknown_values() {
        assert!(output_policy_from_yaml("execution:\n  output: loud\n").is_err());
        assert!(output_policy_from_yaml("execution:\n  output: 1\n").is_err());
    }
}

/// Parses `output:` (on-level default or job-level) into an OutputPolicy;
/// unknown values are rejected loudly.
pub fn output_policy_from_yaml(content: &str) -> Result<OutputPolicy, String> {
    let documents = YamlLoader::load_from_str(content).map_err(|err| err.to_string())?;
    let root = documents
        .first()
        .ok_or_else(|| "Configuration file is empty".to_owned())?;
    output_policy_from_root(root).map_err(|error| match error {
        errors::FzzError::InvalidConfigError(message, _, _) => message,
        other => other.to_string(),
    })
}

#[cfg(test)]
mod service_tests {
    use super::*;

    #[test]
    fn service_defaults_to_false() {
        let rules =
            from_yaml("on:\n  change: '**/*'\njobs:\n  - name: a\n    run: echo a\n").unwrap();
        assert!(!rules[0].service());
    }

    #[test]
    fn service_parses_true() {
        let rules = from_yaml(
            "on:\n  change: '**/*'\njobs:\n  - name: server\n    service: true\n    run: 'sleep 1000'\n",
        )
        .unwrap();
        assert!(rules[0].service());
    }

    #[test]
    fn service_rejects_non_boolean() {
        assert!(from_yaml(
            "on:\n  change: '**/*'\njobs:\n  - name: a\n    service: yes\n    run: echo a\n"
        )
        .is_err());
    }
}

#[cfg(test)]
mod v2_section_tests {
    use super::*;

    const CANONICAL: &str = "on:\n  change: 'src/**'\n  socket: .tmp/fzz.sock\n  debounce: 500ms\nexecution:\n  concurrency: 2\n  output: show-on-failure\nhooks:\n  success: echo ok\n  failure: echo failed\n  close: echo closed\njobs:\n  - name: test\n    run: cargo test\n";

    #[test]
    fn parses_canonical_v2_sections_into_existing_runtime_policies() {
        let rules = from_yaml(CANONICAL).expect("canonical V2 config parses");
        assert_eq!(rules.len(), 1);
        assert_eq!(rules[0].output(), OutputPolicy::ShowOnFailure);
        assert_eq!(concurrency_from_yaml(CANONICAL), Ok(Some(2)));
        assert_eq!(
            control_socket_from_yaml(CANONICAL),
            Ok(Some(".tmp/fzz.sock".to_owned()))
        );
        assert_eq!(
            generation_hooks_from_yaml(CANONICAL)
                .unwrap()
                .success
                .as_deref(),
            Some("echo ok")
        );
        assert_eq!(
            session_hooks_from_yaml(CANONICAL).unwrap().close.as_deref(),
            Some("echo closed")
        );
    }

    #[test]
    fn rejects_old_grouped_v2_placements_instead_of_aliasing_them() {
        for yaml in [
            "on:\n  concurrency: 2\njobs:\n  - name: test\n    run: cargo test\n",
            "on:\n  output: quiet\njobs:\n  - name: test\n    run: cargo test\n",
            "on:\n  success: echo ok\njobs:\n  - name: test\n    run: cargo test\n",
        ] {
            assert!(from_yaml(yaml).is_err(), "old placement must fail: {yaml}");
        }
    }

    #[test]
    fn rejects_unknown_and_wrongly_typed_v2_sections_with_field_paths() {
        let unknown =
            from_yaml("execution:\n  parallelism: 2\njobs:\n  - name: test\n    run: cargo test\n")
                .expect_err("unknown execution property must fail");
        assert!(format!("{unknown:?}").contains("execution.parallelism"));
        let root = from_yaml("unknown: true\njobs:\n  - name: test\n    run: cargo test\n")
            .expect_err("unknown root property must fail");
        assert!(format!("{root:?}").contains("at configuration root"));

        assert_eq!(
            concurrency_from_yaml(
                "execution:\n  concurrency: many\njobs:\n  - name: test\n    run: cargo test\n"
            ),
            Err("Property 'execution.concurrency' must be a positive integer".to_owned())
        );
        assert!(generation_hooks_from_yaml(
            "hooks:\n  success: [echo, ok]\njobs:\n  - name: test\n    run: cargo test\n"
        )
        .is_err());
    }
}

#[cfg(test)]
mod catalog_allowlist_tests {
    use super::*;

    /// Allowlist rejection = an "Invalid property" error; value-shape errors
    /// are separate.
    fn allowlist_rejects(msg: &errors::FzzError) -> bool {
        matches!(
            msg,
            errors::FzzError::InvalidConfigError(m, _, _) if m.contains("Invalid property")
        )
    }

    /// TASK-0094: parser allowlists consume the canonical option catalog
    /// (INIT-TEMPLATE-CONTRACT §10).
    #[test]
    fn on_section_accepts_every_catalog_property() {
        for name in crate::option_catalog::property_names(crate::option_catalog::Owner::On) {
            let yaml = format!(
                "on:\n  change: '**/*'\n  {name}: x\njobs:\n  - name: a\n    run: echo a\n"
            );
            // Keys with a fixed shape accept a probing scalar; the allowlist
            // itself must never reject a catalog property by name.
            let result = from_yaml(&yaml);
            assert!(
                !result.as_ref().is_err_and(allowlist_rejects),
                "{name} must be allowed in 'on'"
            );
        }
    }

    #[test]
    fn on_section_error_lists_every_catalog_property() {
        let err =
            from_yaml("on:\n  change: '**/*'\n  bogus: 1\njobs:\n  - name: a\n    run: echo a\n")
                .expect_err("unknown on property must fail");
        let message = format!("{:?}", err);
        assert!(message.contains("Invalid property 'on.bogus'"));
        for name in crate::option_catalog::property_names(crate::option_catalog::Owner::On) {
            assert!(
                message.contains(name),
                "error must name allowed '{name}': {message}"
            );
        }
    }

    #[test]
    fn job_section_rejects_unknown_properties_actionably() {
        // JOBS-CONFIG-CONTRACT §5: unknown job property must be an actionable
        // error, not a silent accept (schema declares additionalProperties: false).
        let err = from_yaml(
            "on:\n  change: '**/*'\njobs:\n  - name: a\n    run: echo a\n    bogus_key: 1\n",
        )
        .expect_err("unknown job property must fail");
        let message = format!("{:?}", err);
        assert!(message.contains("Invalid property 'bogus_key' in job"));
        assert!(message.contains("a"), "error must name the job: {message}");
    }

    #[test]
    fn job_section_accepts_every_catalog_property() {
        for name in crate::option_catalog::property_names(crate::option_catalog::Owner::Job) {
            let yaml = format!(
                "on:\n  change: '**/*'\njobs:\n  - name: a\n    run: echo a\n    {name}: x\n"
            );
            // Probe with a scalar; value-shape errors are separate from the
            // allowlist and must not be raised here.
            let result = from_yaml(&yaml);
            assert!(
                !result.as_ref().is_err_and(allowlist_rejects),
                "{name} must be allowed in job"
            );
        }
    }
}

#[cfg(test)]
mod manual_trigger_tests {
    use super::from_yaml;

    fn parse(yaml: &str) -> Result<Vec<crate::rules::Rules>, crate::errors::FzzError> {
        from_yaml(yaml)
    }

    #[test]
    fn manual_job_parses_with_empty_effective_surface() {
        let rules = parse(
            "on:\n  change: [\"src/**\"]\njobs:\n  - name: await-remote\n    trigger: manual\n    run: ./await.sh\n",
        )
        .expect("manual job is valid");
        assert_eq!(rules.len(), 1);
        assert!(rules[0].is_manual());
        assert!(rules[0].watch_patterns().is_empty(), "no root inheritance");
        assert!(rules[0].ignore_glob_patterns().is_empty());
        assert!(!rules[0].run_on_init());
    }

    #[test]
    fn manual_rejects_own_change() {
        let err =
            parse("jobs:\n  - name: a\n    trigger: manual\n    run: x\n    change: \"a/**\"\n")
                .expect_err("manual+change must be rejected");
        assert!(err
            .to_string()
            .contains("both 'trigger: manual' and 'change'"));
    }

    #[test]
    fn manual_rejects_own_ignore() {
        let err =
            parse("jobs:\n  - name: a\n    trigger: manual\n    run: x\n    ignore: \"a/**\"\n")
                .expect_err("manual+ignore must be rejected");
        assert!(err
            .to_string()
            .contains("both 'trigger: manual' and 'ignore'"));
    }

    #[test]
    fn manual_rejects_run_on_init() {
        let err =
            parse("jobs:\n  - name: a\n    trigger: manual\n    run: x\n    run_on_init: true\n")
                .expect_err("manual+run_on_init must be rejected");
        assert!(err
            .to_string()
            .contains("'trigger: manual' and 'run_on_init'"));
    }

    #[test]
    fn manual_rejects_service() {
        let err = parse("jobs:\n  - name: a\n    trigger: manual\n    run: x\n    service: true\n")
            .expect_err("manual+service must be rejected");
        assert!(err
            .to_string()
            .contains("'trigger: manual' and 'service: true'"));
    }

    #[test]
    fn manual_allows_recovery_parallel_and_root_on() {
        let rules = parse(
            "on:\n  change: [\"src/**\"]\njobs:\n  - name: a\n    trigger: manual\n    run: x\n    parallel: checks\n    recovery: \"echo fix\"\n",
        )
        .expect("recovery/parallel/root-on are valid with manual");
        assert!(rules[0].recovery_commands().is_some());
        assert_eq!(rules[0].parallel(), Some("checks"));
    }

    #[test]
    fn manual_rejects_unknown_value_and_non_string() {
        let err = parse("jobs:\n  - name: a\n    trigger: cron\n    run: x\n")
            .expect_err("unknown value rejected");
        assert!(err.to_string().contains("must be one of: manual"));
        let err = parse("jobs:\n  - name: a\n    trigger: 5\n    run: x\n")
            .expect_err("non-string rejected");
        assert!(err.to_string().contains("must be the string 'manual'"));
    }

    #[test]
    fn manual_rejected_in_root_list_form() {
        let err = parse("- name: a\n  trigger: manual\n  run: x\n  change: \"a/**\"\n")
            .expect_err("legacy root-list form rejects trigger");
        assert!(err
            .to_string()
            .contains("'trigger' is supported only in preferred V2 jobs"));
    }

    #[test]
    fn manual_rejected_in_grouped_legacy_tasks() {
        let err = parse(
            "on:\n  change: [\"src/**\"]\ntasks:\n  - name: a\n    trigger: manual\n    run: x\n",
        )
        .expect_err("grouped legacy tasks reject trigger");
        assert!(err
            .to_string()
            .contains("'trigger' is supported only in preferred V2 jobs"));
    }

    #[test]
    fn non_manual_jobs_keep_root_inheritance_byte_identically() {
        let rules =
            parse("on:\n  change: [\"src/**\"]\njobs:\n  - name: build\n    run: cargo build\n")
                .expect("unchanged config parses");
        assert!(!rules[0].is_manual());
        assert_eq!(rules[0].watch_patterns(), vec!["src/**".to_string()]);
    }
}

#[cfg(test)]
mod timeout_config_tests {
    use super::from_yaml;
    use std::time::Duration;

    #[test]
    fn timeout_parses_ms_s_m_and_bare_seconds() {
        let rules =
            from_yaml("jobs:\n  - name: a\n    run: x\n    timeout: 200ms\n    change: a/**\n")
                .expect("ms parses");
        assert_eq!(
            rules[0].timeout(),
            Some(std::time::Duration::from_millis(200))
        );

        let rules =
            from_yaml("jobs:\n  - name: a\n    run: x\n    timeout: 45s\n    change: a/**\n")
                .expect("s parses");
        assert_eq!(rules[0].timeout(), Some(std::time::Duration::from_secs(45)));

        let rules =
            from_yaml("jobs:\n  - name: a\n    run: x\n    timeout: 30m\n    change: a/**\n")
                .expect("m parses");
        assert_eq!(
            rules[0].timeout(),
            Some(std::time::Duration::from_secs(1800))
        );

        let rules = from_yaml("jobs:\n  - name: a\n    run: x\n    timeout: 2\n    change: a/**\n")
            .expect("bare = seconds");
        assert_eq!(rules[0].timeout(), Some(std::time::Duration::from_secs(2)));
    }

    #[test]
    fn timeout_rejects_zero_negative_garbage_and_hours() {
        for bad in ["0s", "0", "-5s", "banana", "1h", "1h30m"] {
            let err = from_yaml(&format!(
                "jobs:\n  - name: a\n    run: x\n    change: a/**\n    timeout: {bad}\n"
            ))
            .expect_err(&format!("'{bad}' must be rejected"));
            assert!(
                err.to_string().contains("timeout"),
                "error names the field for '{bad}': {err}"
            );
        }
    }

    #[test]
    fn timeout_rejects_service_and_non_string() {
        let err = from_yaml(
            "jobs:\n  - name: a\n    run: x\n    timeout: 30m\n    service: true\n    change: a/**\n",
        )
        .expect_err("timeout+service rejected");
        assert!(err.to_string().contains("'timeout' and 'service: true'"));

        let err =
            from_yaml("jobs:\n  - name: a\n    run: x\n    change: a/**\n    timeout: true\n")
                .expect_err("non-duration type rejected");
        assert!(err.to_string().contains("duration string"));
    }

    #[test]
    fn timeout_rejected_at_both_legacy_sites() {
        let err = from_yaml("- name: a\n  run: x\n  timeout: 30m\n  change: a/**\n")
            .expect_err("root-list legacy rejects timeout");
        assert!(err
            .to_string()
            .contains("'timeout' is supported only in preferred V2 jobs"));

        let err = from_yaml(
            "on:\n  change: [\"a/**\"]\ntasks:\n  - name: a\n    run: x\n    timeout: 30m\n",
        )
        .expect_err("grouped legacy rejects timeout");
        assert!(err
            .to_string()
            .contains("'timeout' is supported only in preferred V2 jobs"));
    }

    #[test]
    fn absent_timeout_means_unbounded() {
        let rules = from_yaml("jobs:\n  - name: a\n    run: x\n    change: a/**\n").unwrap();
        assert_eq!(rules[0].timeout(), None);
    }

    #[test]
    fn execution_timeout_is_inherited_and_job_override_wins() {
        let rules = from_yaml("execution:\n  timeout: 10m\njobs:\n  - name: a\n    run: x\n  - name: b\n    timeout: 30s\n    run: y\n").unwrap();
        assert_eq!(rules[0].timeout(), Some(Duration::from_secs(600)));
        assert_eq!(rules[1].timeout(), Some(Duration::from_secs(30)));
    }

    #[test]
    fn execution_timeout_does_not_bound_services() {
        let rules = from_yaml(
            "execution:\n  timeout: 10m\njobs:\n  - name: svc\n    service: true\n    run: x\n",
        )
        .unwrap();
        assert_eq!(rules[0].timeout(), None);
    }

    #[test]
    fn execution_timeout_rejects_legacy_and_invalid_values() {
        assert!(from_yaml("execution:\n  timeout: 0\njobs:\n  - name: a\n    run: x\n").is_err());
        assert!(
            from_yaml("execution:\n  timeout: null\njobs:\n  - name: a\n    run: x\n").is_err()
        );
        for sentinel in ["inherit", "unbounded"] {
            assert!(from_yaml(&format!(
                "execution:\n  timeout: {sentinel}\njobs:\n  - name: a\n    run: x\n"
            ))
            .is_err());
        }
        assert!(
            from_yaml("execution:\n  timeout: 10m\ntasks:\n  - name: a\n    run: x\n").is_err()
        );
    }
}
