extern crate glob;
extern crate yaml_rust;

use crate::cli;
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

    pub fn from(yaml: &Yaml) -> Self {
        yaml::validate(yaml, "run");
        yaml::validate(yaml, "change");

        Rules {
            name: yaml::extract_strings(&yaml["name"])[0].clone(),
            commands: yaml::extract_strings(&yaml["run"]),
            watch_patterns: yaml::extract_strings(&yaml["change"]),
            ignore_patterns: yaml::extract_strings(&yaml["ignore"]),
            run_on_init: yaml::extract_bool(&yaml["run_on_init"]),
            yaml: Some(yaml.clone()),
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

        let yaml = self.yaml.clone().expect("Failed to get yaml as instance");

        let ignore_as_yaml_list = match yaml["ignore"].clone() {
            Yaml::Array(_) => format!(
                "  ignore: {}",
                yaml::extract_strings(&yaml["ignore"]).join("\n")
            ),
            _ => "".to_owned(),
        };

        let run_on_init = match yaml["run_on_init"].clone() {
            Yaml::Boolean(_) => format!(
                "  run_on_init: {}",
                yaml::extract_bool(&yaml["run_on_init"])
            ),
            _ => "".to_owned(),
        };

        vec![
            format!(
                "- name: {}",
                yaml::extract_strings(&yaml["name"]).join("\n")
            ),
            format!("  run: {}", yaml::extract_strings(&yaml["run"]).join("\n")),
            format!(
                "  change: {}",
                yaml::extract_strings(&yaml["change"]).join("\n")
            ),
            ignore_as_yaml_list,
            run_on_init,
        ]
        .into_iter()
        .filter(|line| !line.is_empty())
        .collect::<Vec<String>>()
        .join("\n")
    }
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
        .map(|c| c.replace("{{filepath}}", &filepath))
        .map(|c| {
            c.replace(
                "{{relative_filepath}}",
                &filepath.replace(&format!("{}/", &opts.current_dir), ""),
            )
        })
        .collect()
}

pub fn from_yaml(file_content: &str) -> Result<Vec<Rules>, String> {
    let items = match YamlLoader::load_from_str(file_content) {
        Ok(val) => val,
        Err(err) => {
            let lines: Vec<&str> = file_content.lines().collect();
            let marker = err.marker();

            let error_line = if marker.line() > lines.len() {
                lines[lines.len() - 1]
            } else {
                lines[marker.line() - 1]
            };

            return Err(vec![
                format!("Failed to load configuration reason: {}", err),
                "".to_owned(),
                format!("Error line:\n  {}", error_line.trim()),
                "".to_owned(),
                "Debugging:".to_owned(),
                "  - Is the type correct?".to_owned(),
                "  - Is there any missing closing quotes, brackets or braces?".to_owned(),
            ]
            .join("\n"));
        }
    };

    if items.len() == 0 {
        return Err(vec![
            "The config file is invalid!",
            "",
            "Debugging:",
            "  - Did you forget to run 'fzz init'?",
            "  - Did you forget to add a rule?",
        ]
        .join("\n"));
    }

    match items[0] {
        Yaml::Array(ref items) => Ok(items.iter().map(Rules::from).collect()),
        _ => Err("Config file is invalid. At least one rule must be declared.".to_owned()),
    }
}

pub fn from_string(patterns: String, command: String) -> Result<Vec<Rules>, String> {
    let current_dir = match std::env::current_dir() {
        Ok(val) => val,
        Err(err) => {
            return Err(format!("Failed to get current directory {}", err));
        }
    };

    let watches = patterns
        .lines()
        .map(|line| {
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
                    None => {
                        println!(
                            "Warning: Was not possible to convert {} to absolute path",
                            line
                        );

                        String::from("")
                    }
                };

                return full_path_as_str;
            }

            match full_path.to_str() {
                Some(val) => val.to_owned(),
                None => {
                    println!(
                        "Warning: Was not possible to convert {} to absolute path",
                        line
                    );

                    String::from("")
                }
            }
        })
        .collect::<Vec<String>>();

    stdout::info(&format!("watching patterns \n {}", watches.join("\n ")));

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

pub fn from_file(filename: &str) -> Result<Vec<Rules>, String> {
    match File::open(filename) {
        Ok(mut file) => {
            let mut content = String::new();

            if let Err(err) = file.read_to_string(&mut content) {
                return Err(format!(
                    "Invalid config file format '{}'. Details: {}",
                    filename, err
                ));
            }

            return from_yaml(&content);
        }

        Err(err) => Err(format!(
            "File {} cannot be opened. Cause: {}",
            cli::watch::DEFAULT_FILENAME,
            err
        )),
    }
}

pub fn from_default_file_config() -> Result<Vec<Rules>, String> {
    let default_filename = cli::watch::DEFAULT_FILENAME;
    match from_file(default_filename) {
        Ok(rules) => Ok(rules),
        Err(err) => match from_file(&default_filename.replace(".yaml", ".yml")) {
            Ok(rules) => Ok(rules),
            Err(_) => Err(format!("Failed to read default config file {}", err)),
        },
    }
}

fn pattern(pattern: &str) -> Pattern {
    Pattern::new(&format!("**{}", pattern))
        .expect(format!("Invalid glob pattern {}", pattern).as_str())
}

pub fn format_rules(rule: &Vec<Rules>) -> String {
    let mut formatted_rules = String::new();

    for rule in rule {
        formatted_rules.push_str(&format!("{}\n", rule.as_string()));
    }

    formatted_rules
}

#[cfg(test)]
mod tests {
    extern crate yaml_rust;

    use crate::rules::TemplateOptions;

    use self::yaml_rust::YamlLoader;
    use super::from_string;
    use super::from_yaml;
    use super::Rules;
    use super::{commands, template};
    use std::env::current_dir;

    #[test]
    fn it_is_watching_path_tests() {
        let file_content = "
        - name: my tests
          run: 'cargo tests'
          change: 'tests/**'
        ";

        let content = YamlLoader::load_from_str(file_content).unwrap();
        let rule = Rules::from(&content[0][0]);

        assert_eq!(true, rule.watch("tests/foo.rs"));
    }

    #[test]
    fn it_is_not_watching_path_test() {
        let file_content = "
        - name: my tests
          run: 'cargo tests'
          change: 'foo/**'
        ";

        let content = YamlLoader::load_from_str(file_content).unwrap();
        let rule = Rules::from(&content[0][0]);

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
        let rule = Rules::from(&content[0][0]);

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
        let rule = Rules::from(&content[0][0]);

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
        let rule = Rules::from(&content[0][0]);

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
        let rule = Rules::from(&content[0][0]);

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
        let rule = Rules::from(&content[0][0]);

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
        let rule = Rules::from(&content[0][0]);

        let result = rule.commands();
        assert_eq!(vec!["cargo tests"], result);
    }

    #[test]
    #[should_panic]
    fn it_validates_the_run_key() {
        let file_content = "
        - name: my source
          change: 'src/**'
          ignore: ['src/test/**', 'src/tmp/**']
        ";
        let content = YamlLoader::load_from_str(file_content).unwrap();
        Rules::from(&content[0][0]);
    }

    #[test]
    #[should_panic]
    fn it_validates_the_when_change_key() {
        let file_content = "
        - name: my source
          run: make test
          ignore: ['src/test/**', 'src/tmp/**']
        ";
        let content = YamlLoader::load_from_str(file_content).unwrap();
        Rules::from(&content[0][0]);
    }

    fn get_absolute_path(path: &str) -> String {
        let mut absolute_path = current_dir().unwrap();
        absolute_path.push(path);
        absolute_path.to_str().unwrap().to_string()
    }

    #[test]
    fn it_does_not_filters_empty_or_one_character_path() {
        let content = "./foo\n./bar\n.\n./baz\n";
        let rules = from_string(String::from(content), String::from("cargo test")).unwrap();
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
          run: 'cargo tests {{relative_filepath}}'
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
                    filepath: Some("/foo/bar/tests/foo.rs".to_owned()),
                    current_dir: format!("{}", "/foo/bar"),
                },
            ),
            vec![
                "cargo tests tests/foo.rs",
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
                "- name: my tests",
                "  run: cargo tests {{filepath}}",
                "  change: tests/**",
                "  run_on_init: true",
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
            result.err().unwrap(),
            vec![
                "Failed to load configuration reason: while scanning an anchor or alias, did not find expected alphabetic or numeric character at line 8 column 19",
                "",
                "Error line:",
                "  change: **/*",
                "",
                "Debugging:",
                "  - Is the type correct?",
                "  - Is there any missing closing quotes, brackets or braces?",
            ]
            .join("\n")
        );

        let empty_file = "
        ";

        let result = from_yaml(empty_file);
        assert!(result.is_err());
        assert_eq!(
            result.err().unwrap(),
            vec![
                "The config file is invalid!",
                "",
                "Debugging:",
                "  - Did you forget to run 'fzz init'?",
                "  - Did you forget to add a rule?",
            ]
            .join("\n")
        );
    }
}
