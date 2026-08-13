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

    Ok(Rules::new(
        name,
        commands,
        watch_patterns,
        ignore_patterns,
        run_on_init,
    ))
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
                        match parse_hash_format(item) {
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
            parse_hash_format(&items[0])
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

/// Parse the new hash format: { on: {...}, tasks: [...] }
fn parse_hash_format(yaml: &Yaml) -> errors::Result<Vec<Rules>> {
    // Extract the 'tasks' array
    let tasks_yaml = &yaml["tasks"];
    let tasks_array = match tasks_yaml {
        Yaml::Array(ref items) => items,
        Yaml::BadValue => {
            return Err(errors::FzzError::InvalidConfigError(
                "Configuration file is invalid. When using the 'on' format, you must provide a 'tasks' array".to_owned(),
                None,
                Some("Example:\non:\n  change: [\"src/**\"]\ntasks:\n  - name: build\n    run: cargo build".to_owned()),
            ));
        }
        _ => {
            return Err(errors::FzzError::InvalidConfigError(
                format!(
                    "Configuration file is invalid. 'tasks' must be an Array/List, got: {}\n```yaml\n{}\n```",
                    yaml::get_type(tasks_yaml),
                    yaml::yaml_to_string(tasks_yaml, 0),
                ),
                None,
                Some("Make sure 'tasks' is defined as a list of task objects".to_owned()),
            ));
        }
    };

    // Extract common rules from the 'on' section (optional)
    let common_rules = extract_common_rules(&yaml["on"])?;

    // Parse each task and merge with common rules
    let mut rules = vec![];
    for task_yaml in tasks_array {
        match rule_from_with_common(task_yaml, &common_rules) {
            Ok(rule) => rules.push(rule),
            Err(err) => return Err(err),
        }
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
                        if key_str != "change" && key_str != "ignore" && key_str != "socket" {
                            return Err(errors::FzzError::InvalidConfigError(
                                format!(
                                    "Invalid property '{}' in 'on' section. Only 'change', 'ignore', and 'socket' are allowed.",
                                    key_str
                                ),
                                None,
                                Some("Example:\non:\n  change: [\"src/**\"]\n  ignore: [\"**/*.log\"]\n  socket: .tmp/funzzy/control.sock".to_owned()),
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

    Ok(Rules::new(
        name,
        commands,
        watch_patterns,
        ignore_patterns,
        run_on_init,
    ))
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

pub fn from_string(patterns: Vec<String>, command: String) -> errors::Result<Vec<Rules>> {
    let watches = patterns
        .iter()
        .map(|pathline| prepare_as_glob_pattern(pathline))
        .collect::<errors::Result<Vec<String>>>()?;

    let run_on_init = true;
    let ignore = vec![];
    Ok(vec![Rules::new(
        "unnamed".to_owned(),
        vec![command],
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
    use super::control_socket_from_yaml;
    use super::from_string;
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
        let rules = from_string(content, String::from("cargo test")).unwrap();
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
                "tasks:",
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
        assert!(err.contains("'tasks' must be an Array/List"));
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
