extern crate glob;
extern crate yaml_rust;

use crate::cli;
use crate::errors;
use crate::yaml;

use self::glob::Pattern;
use self::yaml_rust::Yaml;
use self::yaml_rust::YamlLoader;
use crate::stdout;
use std::fs::File;
#[warn(unused_imports)]
use std::io::prelude::*;

#[derive(Debug, Clone)]
pub struct Rules {
    pub name: String,

    commands: Vec<String>,
    watch_patterns: Vec<String>,
    ignore_patterns: Vec<String>,
    run_on_init: bool,

    yaml: Option<Yaml>,
}

impl Rules {
    pub fn new(
        name: String,
        commands: Vec<String>,
        watches: Vec<String>,
        ignores: Vec<String>,
        run_on_init: bool,
    ) -> Self {
        Rules {
            name,
            commands,
            watch_patterns: watches,
            ignore_patterns: ignores,
            run_on_init,
            yaml: None,
        }
    }

    pub fn watch(&self, path: &str) -> bool {
        self.watch_relative_paths()
            .iter()
            .any(|watch| pattern(&format!("/{}", watch)).matches(path))
            || self
                .watch_absolute_paths()
                .iter()
                .any(|watch| pattern(watch).matches(path))
    }

    pub fn ignore(&self, path: &str) -> bool {
        self.ignore_patterns.iter().any(|watch| {
            pattern(&format!("/{}", watch)).matches(path)
                || watch.starts_with("/") && pattern(watch).matches(path)
        })
    }

    pub fn commands(&self) -> Vec<String> {
        self.commands.clone()
    }

    pub fn watch_patterns(&self) -> Vec<String> {
        self.watch_patterns.clone()
    }

    pub fn watch_glob_patterns(&self) -> Vec<String> {
        self.watch_patterns.clone()
    }

    pub fn ignore_glob_patterns(&self) -> Vec<String> {
        self.ignore_patterns.clone()
    }

    pub fn run_on_init(&self) -> bool {
        self.run_on_init
    }

    pub fn watch_absolute_paths(&self) -> Vec<String> {
        self.watch_patterns()
            .into_iter()
            .filter(|c| c.starts_with("/"))
            .collect::<Vec<String>>()
    }

    pub fn watch_relative_paths(&self) -> Vec<String> {
        self.watch_patterns()
            .into_iter()
            .filter(|c| !c.starts_with("/"))
            .collect::<Vec<String>>()
    }

    pub fn as_string(&self) -> String {
        if None == self.yaml {
            return "".to_owned();
        }

        yaml::yaml_to_string(self.yaml.as_ref().unwrap(), 0)
    }

    pub fn validate(&self) -> Result<(), String> {
        let name = if self.name.is_empty() {
            "_unnamed_".to_owned()
        } else {
            self.name.clone()
        };

        if self.commands().len() == 0 {
            return Err(format!(
                "Rule '{}' contains no command to run. Empty 'run' property.",
                name
            ));
        }

        if self.watch_patterns().len() == 0 && !self.run_on_init() {
            return Err(format!(
                "Rule '{}' must contain a `change` and/or `run_on_init` property.",
                name
            ));
        }

        for watch_pattern in self.watch_patterns() {
            match Pattern::new(&watch_pattern) {
                Ok(_) => (),
                Err(err) => {
                    return Err(vec![
                        format!(
                            "Rule '{}' contains an invalid `change` glob pattern '{}'.",
                            name, watch_pattern
                        ),
                        format!("  {}", err),
                        "  Read more: https://en.wikipedia.org/wiki/Glob_(programming)".to_owned(),
                    ]
                    .join("\n"));
                }
            }
        }

        for ignore_pattern in self.ignore_patterns.clone() {
            match Pattern::new(&ignore_pattern) {
                Ok(_) => (),
                Err(err) => {
                    return Err(vec![
                        format!(
                            "Rule '{}' contains an invalid `ignore` glob pattern '{}'.",
                            name, ignore_pattern
                        ),
                        format!("  {}", err),
                        "  Read more: https://en.wikipedia.org/wiki/Glob_(programming)".to_owned(),
                    ]
                    .join("\n"));
                }
            }
        }

        Ok(())
    }
}

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

    Ok(Rules {
        name,
        commands,
        watch_patterns,
        ignore_patterns,
        run_on_init,
        yaml: Some(yaml.clone()),
    })
}

pub fn commands(rules: Vec<Rules>) -> Vec<String> {
    rules
        .iter()
        .map(|rule| rule.commands())
        .flat_map(|rule| rule.to_vec())
        .collect::<Vec<String>>()
}

pub struct TemplateOptions {
    pub filepath: Option<String>,
    pub current_dir: String,
}

pub fn template(commands: Vec<String>, opts: TemplateOptions) -> Vec<String> {
    let filepath = match opts.filepath {
        Some(val) => val,
        None => "".to_owned(),
    };

    commands
        .iter()
        .map(|c| {
            if c.contains("{{") {
                c.split("{{")
                    .map(|part| {
                        if part.contains("}}") {
                            let parts: Vec<&str> = part.split("}}").collect();
                            let tpl = parts[0].trim();
                            let rest = parts[1];

                            match tpl {
                                "filepath" | "absolute_path" => format!("{}{}", &filepath, rest),
                                "relative_filepath" | "relative_path" => {
                                    let relative_path =
                                        &filepath.replace(&format!("{}/", &opts.current_dir), "");
                                    format!("{}{}", relative_path, rest)
                                }
                                _ => {
                                    stdout::warn(&format!("Unknown template variable '{}'.", tpl));
                                    format!("{}{}{}{}", "{{", parts[0], "}}", parts[1])
                                }
                            }
                        } else {
                            part.to_owned()
                        }
                    })
                    .collect::<Vec<String>>()
                    .join("")
            } else {
                c.to_owned()
            }
        })
        .collect()
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
                match rule_from(item) {
                    Ok(rule) => rules.push(rule),
                    Err(err) => return Err(err),
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
                        if key_str != "change" && key_str != "ignore" {
                            return Err(errors::FzzError::InvalidConfigError(
                                format!(
                                    "Invalid property '{}' in 'on' section. Only 'change' and 'ignore' are allowed.",
                                    key_str
                                ),
                                None,
                                Some("Example:\non:\n  change: [\"src/**\"]\n  ignore: [\"**/*.log\"]".to_owned()),
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

    // Extract task-specific patterns, or use common rules if not specified
    let watch_patterns_str = match yaml::extract_list(yaml, "change") {
        Ok(patterns) => patterns,
        Err(_) => {
            // Task doesn't define 'change', inherit from common rules
            common.change.clone()
        }
    };

    let ignore_patterns_str = match yaml::extract_list(yaml, "ignore") {
        Ok(patterns) => patterns,
        Err(_) => {
            // Task doesn't define 'ignore', inherit from common rules
            common.ignore.clone()
        }
    };

    let run_on_init = yaml::extract_bool(yaml, "run_on_init");

    let watch_patterns = ensure_glob_only(watch_patterns_str, "change")?;
    let ignore_patterns = ensure_glob_only(ignore_patterns_str, "ignore")?;

    Ok(Rules {
        name,
        commands,
        watch_patterns,
        ignore_patterns,
        run_on_init,
        yaml: Some(yaml.clone()),
    })
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

    stdout::info(&format!("watching patterns\r{}", watches.join("\n")));

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

fn pattern(pattern: &str) -> Pattern {
    Pattern::new(&format!("**{}", pattern)).expect(
        &vec![
            format!("Invalid glob pattern {}", pattern),
            vec![
                "",
                "Some example of valid patterns: ",
                " foo/**/* - Matches any file of any subfolder of foo",
                " *        - Matches any string, of any length",
                " foo*     - Matches any string beginning with foo",
                " *x*      - Matches any string containing an x",
                " *.tar.gz - Matches any string ending with .tar.gz",
                " *.[ch]   - Matches any string ending with .c or .h",
                " foo?     - Matches foot or foo$ but not fools",
            ]
            .join("\n")
            .to_string(),
        ]
        .join("\n"),
    )
}

pub fn format_rules(rule: &Vec<Rules>) -> String {
    let mut formatted_rules = String::new();

    for rule in rule {
        formatted_rules.push_str(&format!("{}\n", rule.as_string()));
    }

    formatted_rules
}

pub fn validate_rules(rule: &Vec<Rules>) -> Result<(), String> {
    for rule in rule {
        if let Err(err) = rule.validate() {
            return Err(err);
        }
    }

    Ok(())
}

pub fn available_targets(rules: Vec<Rules>) -> String {
    let mut output = String::new();
    output.push_str("Available tasks\n");
    output.push_str(&format!(
        "  - {}\n",
        rules
            .iter()
            .cloned()
            .map(|r| r.name)
            .collect::<Vec<String>>()
            .join("\n  - ")
    ));
    output
}

#[cfg(test)]
mod tests {
    extern crate yaml_rust;

    use crate::rules::TemplateOptions;

    use self::yaml_rust::YamlLoader;
    use super::from_string;
    use super::from_yaml;
    use super::rule_from;
    use super::{commands, template};
    use std::env::current_dir;

    #[test]
    fn it_is_watching_path_tests() {
        let file_content = "
        - name: my tests
          run: 'cargo tests'
          change: 'tests/**'

        - name: my tests
          run: 'cargo tests'
          change:
            - 'src/**/*.rs'
            - 'src/**/*.rs?'
            - 'src/**/*.ab[cx]'
        ";

        let content = YamlLoader::load_from_str(file_content).unwrap();
        let rule = rule_from(&content[0][0]).expect("Failed to parse rule");
        let rule2 = rule_from(&content[0][1]).expect("Failed to parse rule");

        assert_eq!(true, rule.watch("tests/foo.rs"));

        // src/**/*.rs
        assert_eq!(true, rule2.watch("src/foo.rsx"));
        assert_eq!(true, rule2.watch("src/bar/foo.rs"));
        assert_eq!(true, rule2.watch("src/bar/foo.rsx"));
        assert_eq!(true, rule2.watch("src/bar/foo.rs3"));
        assert_eq!(true, rule2.watch("src/bar/foo.rs&"));
        assert_eq!(true, rule2.watch("src/bar/foo.abc"));
        assert_eq!(true, rule2.watch("src/bar/foo.abx"));
        // but not
        assert_eq!(false, rule2.watch("src/bar/foo.ab"));
    }

    #[test]
    fn it_is_not_watching_path_test() {
        let file_content = "
        - name: my tests
          run: 'cargo tests'
          change: 'foo/**'
        ";

        let content = YamlLoader::load_from_str(file_content).unwrap();
        let rule = rule_from(&content[0][0]).expect("Failed to parse rule");

        assert_eq!(false, rule.watch("tests/foo.rs"));
    }

    #[test]
    fn it_accepts_run_on_init() {
        let file_content = "
        - name: my tests
          run: 'cargo tests'
          change: 'foo/**'
          run_on_init: true
        ";

        let content = YamlLoader::load_from_str(file_content).unwrap();
        let rule = rule_from(&content[0][0]).expect("Failed to parse rule");

        assert!(rule.run_on_init());
    }

    #[test]
    fn it_accepts_false_for_run_on_init() {
        let file_content = "
        - name: my tests
          run: 'cargo tests'
          change: 'foo/**'
          run_on_init: false
        ";

        let content = YamlLoader::load_from_str(file_content).unwrap();
        let rule = rule_from(&content[0][0]).expect("Failed to parse rule");

        assert!(!rule.run_on_init());
    }

    #[test]
    fn it_defaults_run_on_init_to_false() {
        let file_content = "
        - name: my tests
          run: 'cargo tests'
          change: 'foo/**'
        ";

        let content = YamlLoader::load_from_str(file_content).unwrap();
        let rule = rule_from(&content[0][0]).expect("Failed to parse rule");

        assert!(!rule.run_on_init());
    }

    #[test]
    fn it_is_ignoring_path_tests() {
        let file_content = "
        - name: my tests
          run: 'cargo tests'
          change: 'bla/**'
          ignore: 'tests/**'
        ";

        let content = YamlLoader::load_from_str(file_content).unwrap();
        let rule = rule_from(&content[0][0]).expect("Failed to parse rule");

        assert_eq!(true, rule.ignore("tests/foo.rs"));
    }

    #[test]
    fn it_is_not_ignoring_path_test() {
        let file_content = "
        - name: my tests
          run: 'cargo tests'
          change: 'bla/**'
          change: 'foo/**'
        ";

        let content = YamlLoader::load_from_str(file_content).unwrap();
        let rule = rule_from(&content[0][0]).expect("Failed to parse rule");

        assert_eq!(false, rule.ignore("tests/foo.rs"));
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
    fn it_replaces_filepath_tpl_with_absolute_filepath() {
        let file_content = "
        - name: my tests
          run: 'cargo tests {{filepath}}'
          change: 'tests/**'

        - name: my tests
          run: ['echo {{filepath}}', 'make tests {{filepath}}']
          change: 'tests/**'
        ";

        let rules = from_yaml(file_content).expect("Failed to parse yaml");

        assert_eq!(
            template(
                commands(rules.clone()),
                TemplateOptions {
                    filepath: Some("tests/foo.rs".to_owned()),
                    current_dir: format!("{}", "/foo/bar"),
                },
            ),
            vec![
                "cargo tests tests/foo.rs",
                "echo tests/foo.rs",
                "make tests tests/foo.rs"
            ]
        );

        assert_eq!(
            template(
                commands(rules.clone()),
                TemplateOptions {
                    filepath: Some("/bar/baz/tests/foo.rs".to_owned()),
                    current_dir: format!("{}", "/foo/bar"),
                },
            ),
            vec![
                "cargo tests /bar/baz/tests/foo.rs",
                "echo /bar/baz/tests/foo.rs",
                "make tests /bar/baz/tests/foo.rs"
            ]
        );
    }

    #[test]
    fn it_replaces_relative_filepath_tpl_with_relative_filepath() {
        let file_content = "
        - name: my tests
          run:
            - 'cargo tests {{relative_filepath}}'
            - 'git add {{relative_path}}'
          change: 'tests/**'

        - name: my tests
          run:
            - 'echo {{filepath}}'
            - 'make tests {{absolute_path}}'
          change: 'tests/**'
        ";

        let rules = from_yaml(file_content).expect("Failed to parse yaml");

        assert_eq!(
            template(
                commands(rules.clone()),
                TemplateOptions {
                    filepath: Some("/foo/bar/tests/foo.rs".to_owned()),
                    current_dir: format!("{}", "/foo/bar"),
                },
            ),
            vec![
                "cargo tests tests/foo.rs",
                "git add tests/foo.rs",
                "echo /foo/bar/tests/foo.rs",
                "make tests /foo/bar/tests/foo.rs"
            ]
        );
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
            rules[0].as_string(),
            vec![
                "name: my tests",
                "run: cargo tests {{filepath}}",
                "change: tests/**",
                "run_on_init: true",
            ]
            .join("\n"),
            "Failed to format rule as string {}",
            rules[0].as_string()
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
    fn it_validates_the_given_glob_patterns_paths() {
        let rules_yaml = from_yaml(
            "
        - name: this is valid
          run: 'cargo tests'
          change:
            - '**/*'
            - '**/*.go'
          ignore:
            - '**/*.log'

        - name: this is an invalid pattern
          run: 'echo invalid'
          change:
            - '**/foo_**.go'
          ignore:
            - '**/*.log'

        - name: this is an invalid pattern 2
          run: 'echo invalid'
          change:
            - '**/*.go'
          ignore:
            - '**/**.*'

        - name: missing trigger property
          run: 'echo invalid'
          ignore: '**/*.go'
        ",
        );
        assert!(rules_yaml.is_ok());

        let rules = rules_yaml.unwrap();
        let first_rule = &rules[0];
        assert!(first_rule.validate().is_ok());

        // The invalid pattern rules
        let second_rule = &rules[1];
        assert!(second_rule.validate().is_err());
        assert_eq!(
            second_rule.validate().err().unwrap(),
            "Rule 'this is an invalid pattern' contains an invalid `change` glob pattern '**/foo_**.go'.
  Pattern syntax error near position 6: recursive wildcards must form a single path component
  Read more: https://en.wikipedia.org/wiki/Glob_(programming)"
        );

        let third_rule = &rules[2];
        assert!(third_rule.validate().is_err());
        assert_eq!(
            third_rule.validate().err().unwrap(),
            "Rule 'this is an invalid pattern 2' contains an invalid `ignore` glob pattern '**/**.*'.
  Pattern syntax error near position 5: recursive wildcards must form a single path component
  Read more: https://en.wikipedia.org/wiki/Glob_(programming)"
        );

        let fourth_rule = &rules[3];
        assert!(fourth_rule.validate().is_err());
        assert_eq!(
            fourth_rule.validate().err().unwrap(),
            "Rule 'missing trigger property' must contain a `change` and/or `run_on_init` property."
        );
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
    fn it_allows_tasks_to_override_common_change() {
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

        // First task inherits common change
        assert_eq!(rules[0].watch_patterns(), vec!["src/**"]);
        assert_eq!(rules[0].ignore_glob_patterns(), vec!["**/*.log"]);

        // Second task overrides change but inherits ignore
        assert_eq!(rules[1].watch_patterns(), vec!["tests/**"]);
        assert_eq!(rules[1].ignore_glob_patterns(), vec!["**/*.log"]);
    }

    #[test]
    fn it_allows_tasks_to_override_common_ignore() {
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

        // Task overrides ignore but inherits change
        assert_eq!(rules[0].watch_patterns(), vec!["src/**"]);
        assert_eq!(
            rules[0].ignore_glob_patterns(),
            vec!["**/*.tmp", "target/**"]
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
}
