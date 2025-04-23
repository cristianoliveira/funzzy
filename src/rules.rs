extern crate glob;
extern crate yaml_rust;

use ignore::gitignore::Gitignore;

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
            // FIXME this is clearly slow and can be optimized
            if watch.starts_with("{{gitignore:") {
                let gitignore_path = watch.replace("{{gitignore:", "").replace("}}", "");
                let gitignore = gitignore_path.replace("{{", "");
                let (gitignore, err) = Gitignore::new(gitignore);
                if let Some(e) = err {
                    stdout::error(&format!(
                        "Failed to load gitignore file: {}. Error: {}",
                        gitignore_path, e
                    ));
                    return false;
                }

                return gitignore.matched(path, false).is_ignore();
            }

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

pub fn load_list_from_dynamic_pattern(dynamic_pattern: Vec<String>) -> Option<Vec<String>> {
    dynamic_pattern
        .iter()
        .filter(|s| s.contains("{{file:")) // Check if the pattern {{file:/filepath/path}} is present
        .cloned()
        .collect::<Vec<String>>()
        .into_iter()
        .map(|dynamic_pattern| load_list_from_file(dynamic_pattern))
        .filter(|s| s.is_some())
        .map(|s| s.unwrap())
        .next()
}

pub fn load_list_from_file(dynamic_pattern: String) -> Option<Vec<String>> {
    if dynamic_pattern.is_empty() {
        return None;
    }
    // Check if the pattern {{file:/filepath/path}} is present
    if !dynamic_pattern.contains("{{file:") {
        return None;
    }

    let filepath = dynamic_pattern
        .replace("{{file:", "")
        .replace("}}", "")
        .replace("{{", "");

    let mut content = String::new();
    match File::open(filepath.clone()) {
        Ok(mut file) => {
            if let Err(err) = file.read_to_string(&mut content) {
                stdout::error(&format!(
                    "Couldn't read configuration file: '{}'. Reason: {}",
                    filepath, err
                ));
                return None;
            }

            Some(
                content
                    .lines()
                    .map(|s| s.to_owned())
                    .filter(|s| !s.is_empty())
                    .filter(|s| !s.starts_with("{{file:"))
                    .collect(),
            )
        }
        Err(err) => {
            stdout::error(&format!(
                "Couldn't open configuration file: '{}'. Reason: {}",
                filepath, err
            ));

            return None;
        }
    }
}

pub fn rule_from(yaml: &Yaml) -> errors::Result<Rules> {
    let name = yaml::extract_string(yaml, "name")?;
    let commands = yaml::extract_list(yaml, "run")?;
    let watch_patterns = yaml::extract_list(yaml, "change").unwrap_or_default();
    let mut ignore_patterns = yaml::extract_list(yaml, "ignore").unwrap_or_default();
    let run_on_init = yaml::extract_bool(yaml, "run_on_init");

    if let Some(dynamic_ignore_patterns) = load_list_from_dynamic_pattern(ignore_patterns.clone()) {
        ignore_patterns.extend(dynamic_ignore_patterns);
        ignore_patterns = ignore_patterns
            .iter()
            .filter(|s| !s.is_empty())
            .filter(|s| !s.starts_with("{{file:"))
            .cloned()
            .collect::<Vec<String>>();
    }

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

    use crate::stdout;
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

    #[test]
    fn it_loads_() {
        // FIXME: this isn't much of a unit test
        // Check the file examples/workdir/.ignored for this
        let file_content = "
        - name: my test
          run: 'cargo tests'
          change: 'examples/**'
          ignore: '{{file:examples/workdir/.ignored}}'
        ";

        let content = YamlLoader::load_from_str(file_content).unwrap();
        let rule = rule_from(&content[0][0]).unwrap();

        let result = rule.commands();
        assert_eq!(vec!["cargo tests"], result);
        assert_eq!(rule.ignore_patterns.len(), 2); // See the file
        assert!(rule.ignore("examples/workdir/ignored/test.rs"));
        assert!(rule.ignore("examples/workdir/ignored/test2.txt"));
        assert!(
            rule.ignore("examples/workdir/test.foo"),
            "examples/workdir/test.foo must be ignored"
        );
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

        let empty_file = "
        on:
            - name: foo
              run: echo foo
        ";

        let result = from_yaml(empty_file);
        assert!(result.is_err());
        assert_eq!(
            result.err().unwrap().to_string(),
            vec![
                "Configuration file is invalid. Expected an Array/List of rules got: Hash",
                "```yaml",
                "on:",
                "  - name: foo",
                "    run: echo foo",
                "```",
                "Hint: Make sure to declare the rules as a list without any root property",
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
}
