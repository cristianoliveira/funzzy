//! Configuration loading: YAML parsing, compatibility formats, and filesystem
//! adapters that turn configuration into task models (`rules::Rules`).
//!
//! Parser DTOs (common-rule groups) and retained YAML presentation live here;
//! the task model in `rules.rs` stays a pure user-facing value with no YAML
//! knowledge. Legacy list, grouped `on`/`tasks`, and nested group formats are
//! all accepted here exactly as documented.

extern crate yaml_rust;

use crate::cli;
use crate::errors;
use crate::rules::Rules;
use crate::yaml;

use self::yaml_rust::Yaml;
use self::yaml_rust::YamlLoader;
use std::fs::File;
use std::io::prelude::*;
use std::time::Duration;
pub fn rule_from(yaml: &Yaml) -> errors::Result<Rules> {
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

    if items.len() == 0 {
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
                        match parse_hash_format(item, true) {
                            Ok(group_rules) => rules.extend(group_rules),
                            Err(err) => return Err(err),
                        }
                    }
                    _ => {
                        // This is a regular task
                        match rule_from(item) {
                            Ok(rule) => rules.push(rule),
                            Err(err) => return Err(err),
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

    // Extract common rules from the 'on' section (optional)
    let common_rules = extract_common_rules(&yaml["on"])?;

    // Parse each task and merge with common rules; duplicate names are a
    // config bug (TASK-0075/0076), never a silent merge or reorder.
    let mut rules = vec![];
    let mut seen: std::collections::HashSet<String> = std::collections::HashSet::new();
    for task_yaml in tasks_array {
        let rule = match rule_from_with_common(task_yaml, &common_rules) {
            Ok(rule) => rule,
            Err(err) => return Err(err),
        };
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
            })
        }
        Yaml::Hash(_) => {
            let change = yaml::extract_list(yaml, "change").unwrap_or_default();
            let ignore = yaml::extract_list(yaml, "ignore").unwrap_or_default();

            // Validate that only allowed properties are present
            if let Yaml::Hash(ref hash) = yaml {
                for (key, _) in hash {
                    if let Yaml::String(ref key_str) = key {
                        if key_str != "change"
                            && key_str != "ignore"
                            && key_str != "socket"
                            && key_str != "concurrency"
                            && key_str != "debounce"
                            && key_str != "watch_backend"
                            && key_str != "poll_interval"
                            && key_str != "respect_gitignore"
                            && key_str != "success"
                            && key_str != "failure"
                        {
                            return Err(errors::FzzError::InvalidConfigError(
                                format!(
                                    "Invalid property '{}' in 'on' section. Only 'change', 'ignore', 'socket', 'concurrency', 'debounce', 'watch_backend', 'poll_interval', and 'respect_gitignore' are allowed.",
                                    key_str
                                ),
                                None,
                                Some("Example:\non:\n  change: [\"src/**\"]\n  ignore: [\"**/*.log\"]\n  socket: .tmp/funzzy/control.sock\n  concurrency: 2\n  debounce: 500ms".to_owned()),
                            ));
                        }
                    }
                }
            }

            Ok(CommonRules {
                change: ensure_glob_only(change, "on.change")?,
                ignore: ensure_glob_only(ignore, "on.ignore")?,
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
    let name = yaml::extract_string(yaml, "name")?;
    let commands = yaml::extract_list(yaml, "run")?;

    // Tasks EXTEND the shared `on` rules; they never replace them. A task's
    // own `change` and `ignore` are appended to (and deduped against) the
    // common patterns, so root-level scope and safety rails always apply.
    let task_change = yaml::extract_list(yaml, "change").unwrap_or_default();
    let task_ignore = yaml::extract_list(yaml, "ignore").unwrap_or_default();

    let watch_patterns = ensure_glob_only(merge_patterns(&common.change, task_change), "change")?;
    let ignore_patterns = ensure_glob_only(merge_patterns(&common.ignore, task_ignore), "ignore")?;

    let run_on_init = yaml::extract_bool(yaml, "run_on_init");
    let parallel = yaml::extract_optional_string(yaml, "parallel")?;
    let cwd = yaml::extract_optional_string(yaml, "cwd")?;
    let environment = yaml::extract_optional_string_map(yaml, "env")?;

    let rule = Rules::new(name, commands, watch_patterns, ignore_patterns, run_on_init)
        .with_execution_context(cwd, environment)
        .with_inherited_patterns(inherited_patterns(common));
    Ok(match parallel {
        Some(group) => rule.with_parallel(group),
        None => rule,
    })
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
        line_number = line_number + 1;
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
                        vec![
                        "When using stdin, make sure to provide a list of valid files or directories.",
                        "The output of command `find` is a good example",
                        ].join("\n"),
                    ),
                ));
            }
        }
    }

    return Ok(watches);
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

/// Reads optional global `on.concurrency` cap. Missing yields `Ok(None)`
/// (caller decides default); zero, negative, and non-integer values fail.
pub fn concurrency_from_yaml(content: &str) -> Result<Option<usize>, String> {
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

    if on["concurrency"] == Yaml::BadValue {
        return Ok(None);
    }

    match &on["concurrency"] {
        Yaml::Integer(value) if *value > 0 => Ok(Some(*value as usize)),
        Yaml::Integer(_) => Err(
            "Property 'on.concurrency' must be a positive integer (got zero or negative)"
                .to_owned(),
        ),
        _ => Err("Property 'on.concurrency' must be a positive integer".to_owned()),
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
            "invalid 'on.debounce' duration '{}': expected <number> with optional ms/s/m suffix (bare number = seconds)",
            raw
        )
    })?;
    if value == 0 {
        return Err(format!(
            "invalid 'on.debounce' duration '{}': must be positive",
            raw
        ));
    }
    let millis = value
        .checked_mul(multiplier)
        .ok_or_else(|| format!("invalid 'on.debounce' duration '{}': too large", raw))?;
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

            return from_yaml(&content);
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
    extern crate yaml_rust;

    use self::yaml_rust::YamlLoader;
    use super::concurrency_from_yaml;
    use super::control_socket_from_yaml;
    use super::from_argv;
    use super::from_yaml;
    use super::rule_as_yaml;
    use super::rule_from;
    use std::env::current_dir;

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
          change: 'foo/**'
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
            vec![
                "name: my tests",
                "run: cargo tests {{filepath}}",
                "change: tests/**",
                "run_on_init: true",
            ]
            .join("\n"),
            "Failed to format rule as string {}",
            rule_as_yaml(&rules[0])
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
            vec![
                "Failed to load configuration at line:",
                "|           run: 'cargo tests'",
                "|>          change: **/*",
                "|         ",
                "Reason: while scanning an anchor or alias, did not find expected alphabetic or numeric character at line 8 column 19",
                "Hint: Check for wrong types, any missing quotes for glob pattern or incorrect identation",
            ]
            .join("\n")
        );

        let empty_file = "
        ";

        let result = from_yaml(empty_file);
        assert!(result.is_err());
        assert_eq!(
            result.err().unwrap().to_string(),
            vec![
                "Configuration file is invalid! There are no rules to watch",
                "Hint: Make sure to declare at least one rule. Try to run `fzz init` to generate a new configuration from scratch",
            ]
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
            vec![
                "Configuration file is invalid. When using the 'on' format, you must provide a 'tasks' array",
                "Hint: Example:",
                "on:",
                "  change: [\"src/**\"]",
                "jobs:",
                "  - name: build",
                "    run: cargo build",
            ]
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
        assert!(err.contains("Invalid property 'invalid_prop' in 'on' section"));
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
    fn it_accepts_concurrency_from_on_section() {
        let file_content = "
on:\n  concurrency: 4\n  change: 'src/**'\ntasks:\n  - name: lint\n    run: make lint\n";
        assert_eq!(concurrency_from_yaml(file_content).unwrap(), Some(4));
    }

    #[test]
    fn it_rejects_jobs_so_name_remains_available_for_workflow_items() {
        let file_content = "\non:\n  jobs: 4\ntasks:\n  - name: lint\n    run: make lint\n";
        let error = from_yaml(file_content).expect_err("on.jobs is not a supported alias");
        assert!(
            error.to_string().contains("Invalid property 'jobs'"),
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
        let zero = "\non:\n  concurrency: 0\ntasks:\n  - name: a\n    run: echo a\n";
        let negative = "\non:\n  concurrency: -2\ntasks:\n  - name: a\n    run: echo a\n";

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
        let file_content = "\non:\n  concurrency: many\ntasks:\n  - name: a\n    run: echo a\n";
        let err = concurrency_from_yaml(file_content).expect_err("string concurrency must fail");
        assert!(err.contains("positive integer"), "unexpected: {}", err);
    }

    #[test]
    fn it_rejects_concurrency_outside_object_on() {
        let file_content = "\non: 3\ntasks:\n  - name: a\n    run: echo a\n";
        let err = concurrency_from_yaml(file_content).expect_err("scalar on must fail");
        assert!(
            err.contains("'on' must be an object"),
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
        assert_eq!(
            respect_gitignore_from_yaml("on:\n  change: '**/*'\n").unwrap(),
            false
        );
    }

    #[test]
    fn respect_gitignore_parses_boolean() {
        assert_eq!(
            respect_gitignore_from_yaml("on:\n  respect_gitignore: true\n").unwrap(),
            true
        );
        assert_eq!(
            respect_gitignore_from_yaml("on:\n  respect_gitignore: false\n").unwrap(),
            false
        );
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
        let hooks = hooks_from_yaml("on:\n  change: '**/*'\n").unwrap();
        assert_eq!(hooks.success, None);
        assert_eq!(hooks.failure, None);
    }

    #[test]
    fn hooks_parse_success_and_failure_commands() {
        let hooks = hooks_from_yaml(
            "on:\n  change: '**/*'\n  success: 'echo done > done.txt'\n  failure: 'echo failed > failed.txt'\n",
        )
        .unwrap();
        assert_eq!(hooks.success.as_deref(), Some("echo done > done.txt"));
        assert_eq!(hooks.failure.as_deref(), Some("echo failed > failed.txt"));
    }

    #[test]
    fn hooks_reject_non_string_values() {
        assert!(hooks_from_yaml("on:\n  success: [a, b]\n").is_err());
        assert!(hooks_from_yaml("on:\n  failure: 1\n").is_err());
    }
}

/// Parsed run-level hooks (`on.success` / `on.failure`), TASK-0040.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct RunHooks {
    pub success: Option<String>,
    pub failure: Option<String>,
}

/// Parses `on.success` / `on.failure` hook commands; absent = None,
/// non-string values are rejected loudly.
pub fn hooks_from_yaml(content: &str) -> Result<RunHooks, String> {
    let documents = YamlLoader::load_from_str(content).map_err(|err| err.to_string())?;
    let root = documents
        .first()
        .ok_or_else(|| "Configuration file is empty".to_owned())?;
    let on = &root["on"];
    if on == &Yaml::BadValue {
        return Ok(RunHooks::default());
    }
    if !matches!(on, Yaml::Hash(_)) {
        return Err("Property 'on' must be an object".to_owned());
    }
    let read_hook = |key: &str| -> Result<Option<String>, String> {
        match &on[key] {
            Yaml::BadValue => Ok(None),
            Yaml::String(value) => Ok(Some(value.clone())),
            _ => Err(format!("Property 'on.{}' must be a command string", key)),
        }
    };
    Ok(RunHooks {
        success: read_hook("success")?,
        failure: read_hook("failure")?,
    })
}

pub fn hooks_from_file(filename: &str) -> Result<RunHooks, String> {
    let mut file = File::open(filename).map_err(|err| err.to_string())?;
    let mut content = String::new();
    file.read_to_string(&mut content)
        .map_err(|err| err.to_string())?;
    hooks_from_yaml(&content)
}
