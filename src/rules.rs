//! User-facing task model.
//!
//! `Rules` is a pure domain value: one configured task with its commands,
//! watch/ignore patterns, and init behavior. Matching semantics, glob
//! validation, and task-level presentation live here. YAML parsing,
//! compatibility formats, and file loading live in `config`; command template
//! expansion lives in `template`.

extern crate glob;

use self::glob::Pattern;

#[derive(Debug, Clone)]
pub struct Rules {
    pub name: String,

    commands: Vec<String>,
    watch_patterns: Vec<String>,
    ignore_patterns: Vec<String>,
    run_on_init: bool,
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
        }
    }

    pub fn watch(&self, path: &str) -> bool {
        self.watch_relative(path) || self.watch_absolute(path)
    }

    pub fn watch_relative(&self, path: &str) -> bool {
        let normalized_path = if path.starts_with('/') {
            path.to_owned()
        } else {
            format!("/{}", path)
        };

        self.watch_relative_paths().iter().any(|watch| {
            let normalized = watch.trim_start_matches("./");
            anchored_pattern(&format!("/{}", normalized)).matches(&normalized_path)
        })
    }

    pub fn watch_absolute(&self, path: &str) -> bool {
        self.watch_absolute_paths()
            .iter()
            .any(|watch| anchored_pattern(watch).matches(path))
    }

    pub fn ignore(&self, path: &str) -> bool {
        self.ignore_relative(path) || self.ignore_absolute(path)
    }

    pub fn ignore_relative(&self, path: &str) -> bool {
        let normalized_path = if path.starts_with('/') {
            path.to_owned()
        } else {
            format!("/{}", path)
        };

        self.ignore_patterns
            .iter()
            .filter(|pattern| !pattern.starts_with("/"))
            .any(|ignore| {
                let normalized = ignore.trim_start_matches("./");
                anchored_pattern(&format!("/{}", normalized)).matches(&normalized_path)
            })
    }

    pub fn ignore_absolute(&self, path: &str) -> bool {
        self.ignore_patterns
            .iter()
            .filter(|pattern| pattern.starts_with("/"))
            .any(|ignore| anchored_pattern(ignore).matches(path))
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

pub fn commands(rules: Vec<Rules>) -> Vec<String> {
    rules
        .iter()
        .map(|rule| rule.commands())
        .flat_map(|rule| rule.to_vec())
        .collect::<Vec<String>>()
}

fn create_pattern(pattern: &str, anchored: bool) -> Pattern {
    let compiled_pattern = if anchored {
        pattern.to_owned()
    } else {
        format!("**{}", pattern)
    };

    Pattern::new(&compiled_pattern).expect(
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

fn anchored_pattern(pattern: &str) -> Pattern {
    create_pattern(pattern, true)
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
#[cfg(test)]
mod tests {
    use super::Rules;
    use crate::config;

    fn rule(
        name: &str,
        commands: &[&str],
        watches: &[&str],
        ignores: &[&str],
        run_on_init: bool,
    ) -> Rules {
        Rules::new(
            name.to_owned(),
            commands.iter().map(|s| s.to_string()).collect(),
            watches.iter().map(|s| s.to_string()).collect(),
            ignores.iter().map(|s| s.to_string()).collect(),
            run_on_init,
        )
    }

    #[test]
    fn it_is_watching_path_tests() {
        let first = rule("my tests", &["cargo tests"], &["tests/**"], &[], false);
        let second = rule(
            "my tests",
            &["cargo tests"],
            &["src/**/*.rs", "src/**/*.rs?", "src/**/*.ab[cx]"],
            &[],
            false,
        );

        assert_eq!(true, first.watch("tests/foo.rs"));

        // src/**/*.rs
        assert_eq!(true, second.watch("src/foo.rsx"));
        assert_eq!(true, second.watch("src/bar/foo.rs"));
        assert_eq!(true, second.watch("src/bar/foo.rsx"));
        assert_eq!(true, second.watch("src/bar/foo.rs3"));
        assert_eq!(true, second.watch("src/bar/foo.rs&"));
        assert_eq!(true, second.watch("src/bar/foo.abc"));
        assert_eq!(true, second.watch("src/bar/foo.abx"));
        // but not
        assert_eq!(false, second.watch("src/bar/foo.ab"));
    }

    #[test]
    fn it_is_not_watching_path_test() {
        let rule = rule("my tests", &["cargo tests"], &["foo/**"], &[], false);

        assert_eq!(false, rule.watch("tests/foo.rs"));
    }

    #[test]
    fn it_accepts_run_on_init() {
        let rule = rule("my tests", &["cargo tests"], &["foo/**"], &[], true);

        assert!(rule.run_on_init());
    }

    #[test]
    fn it_accepts_false_for_run_on_init() {
        let rule = rule("my tests", &["cargo tests"], &["foo/**"], &[], false);

        assert!(!rule.run_on_init());
    }

    #[test]
    fn it_defaults_run_on_init_to_false() {
        let rule = rule("my tests", &["cargo tests"], &["foo/**"], &[], false);

        assert!(!rule.run_on_init());
    }

    #[test]
    fn it_is_ignoring_path_tests() {
        let rule = rule(
            "my tests",
            &["cargo tests"],
            &["bla/**"],
            &["tests/**"],
            false,
        );

        assert_eq!(true, rule.ignore("tests/foo.rs"));
    }

    #[test]
    fn it_is_not_ignoring_path_test() {
        let rule = rule(
            "my tests",
            &["cargo tests"],
            &["bla/**", "foo/**"],
            &[],
            false,
        );

        assert_eq!(false, rule.ignore("tests/foo.rs"));
    }

    #[test]
    fn it_validates_the_given_glob_patterns_paths() {
        let rules_yaml = config::from_yaml(
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
