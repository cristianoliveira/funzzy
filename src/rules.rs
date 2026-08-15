//! User-facing task model.
//!
//! `Rules` is a pure domain value: one configured task with its commands,
//! watch/ignore patterns, and init behavior. Matching semantics, glob
//! validation, and task-level presentation live here. YAML parsing,
//! compatibility formats, and file loading live in `config`; command template
//! expansion lives in `template`.

extern crate glob;

use self::glob::Pattern;
use std::collections::BTreeMap;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Rules {
    pub name: String,

    commands: Vec<String>,
    /// When present, the rule is an ad-hoc `exec` command: this exact argv
    /// is spawned directly without a shell. Mutually exclusive with
    /// `commands` (an argv rule has no shell command list).
    argv: Option<Vec<String>>,
    watch_patterns: Vec<String>,
    ignore_patterns: Vec<String>,
    run_on_init: bool,
    /// Optional named `parallel` group (TASK-0024/0025). `None` means the
    /// task is serial and never runs concurrently with siblings.
    parallel: Option<String>,
    /// Optional task working directory, resolved against workspace root when
    /// building executable plan.
    cwd: Option<String>,
    /// Per-task environment overlay. Values are never applied globally.
    environment: BTreeMap<String, String>,
    /// Patterns inherited from a group `on` section (TASK-0023). Diagnostics
    /// use this to report `rule_origin=group` vs `task` for the effective
    /// rule responsible for a decision. Empty for plain task lists.
    inherited: Vec<String>,
    /// Per-job output policy (TASK-0041); Inherit is the default.
    output: crate::config::OutputPolicy,
    /// Managed long-running service task (TASK-0035); opt-in, default false.
    service: bool,
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
            argv: None,
            watch_patterns: watches,
            ignore_patterns: ignores,
            run_on_init,
            parallel: None,
            cwd: None,
            environment: BTreeMap::new(),
            inherited: vec![],
            output: crate::config::OutputPolicy::Inherit,
            service: false,
        }
    }

    /// Creates an ad-hoc `exec` rule whose single command is an exact argv
    /// vector. The program and its arguments cross parser/runtime boundaries
    /// without being joined and re-parsed through a shell.
    pub fn from_argv(
        name: String,
        argv: Vec<String>,
        watches: Vec<String>,
        ignores: Vec<String>,
        run_on_init: bool,
    ) -> Self {
        Rules {
            name,
            commands: vec![],
            argv: Some(argv),
            watch_patterns: watches,
            ignore_patterns: ignores,
            run_on_init,
            parallel: None,
            cwd: None,
            environment: BTreeMap::new(),
            inherited: vec![],
            output: crate::config::OutputPolicy::Inherit,
            service: false,
        }
    }

    /// Adds task-local process context without mutating parent process.
    pub fn with_execution_context(
        mut self,
        cwd: Option<String>,
        environment: BTreeMap<String, String>,
    ) -> Self {
        self.cwd = cwd;
        self.environment = environment;
        self
    }

    pub fn cwd(&self) -> Option<&str> {
        self.cwd.as_deref()
    }

    pub fn environment(&self) -> &BTreeMap<String, String> {
        &self.environment
    }

    /// Marks patterns inherited from a group `on` section (TASK-0023): the
    /// effective rule origin for diagnostics. Set by the config parser when
    /// merging group scope into tasks.
    pub fn with_inherited_patterns(mut self, inherited: Vec<String>) -> Self {
        self.inherited = inherited;
        self
    }

    /// Patterns inherited from a group `on` section, if any.
    pub fn inherited_patterns(&self) -> &[String] {
        &self.inherited
    }

    /// Declares the task as a member of the named `parallel` group. Only
    /// consecutive tasks sharing the same non-empty group name run together.
    /// Sets the per-job output policy (TASK-0041).
    pub fn with_output(mut self, output: crate::config::OutputPolicy) -> Self {
        self.output = output;
        self
    }

    /// The per-job output policy.
    pub fn output(&self) -> crate::config::OutputPolicy {
        self.output
    }

    /// Marks the job as a managed long-running service (TASK-0035).
    pub fn with_service(mut self, service: bool) -> Self {
        self.service = service;
        self
    }

    /// True when the job is a managed long-running service.
    pub fn service(&self) -> bool {
        self.service
    }

    pub fn with_parallel(mut self, group: String) -> Self {
        self.parallel = Some(group);
        self
    }

    /// The optional named `parallel` group this task belongs to.
    pub fn parallel(&self) -> Option<&str> {
        self.parallel.as_deref()
    }

    pub fn watch(&self, path: &str) -> bool {
        self.watch_relative(path) || self.watch_absolute(path)
    }

    pub fn watch_relative(&self, path: &str) -> bool {
        !self.watch_relative_patterns(path).is_empty()
    }

    /// Change patterns that match `path` through the relative matcher.
    pub fn watch_relative_patterns(&self, path: &str) -> Vec<String> {
        let normalized_path = if path.starts_with('/') {
            path.to_owned()
        } else {
            format!("/{}", path)
        };

        self.watch_relative_paths()
            .iter()
            .filter(|watch| {
                let normalized = watch.trim_start_matches("./");
                anchored_pattern(&format!("/{}", normalized)).matches(&normalized_path)
            })
            .cloned()
            .collect()
    }

    pub fn watch_absolute(&self, path: &str) -> bool {
        !self.watch_absolute_patterns(path).is_empty()
    }

    /// Change patterns that match `path` through the absolute matcher.
    pub fn watch_absolute_patterns(&self, path: &str) -> Vec<String> {
        self.watch_absolute_paths()
            .iter()
            .filter(|watch| anchored_pattern(watch).matches(path))
            .cloned()
            .collect()
    }

    /// Change patterns that match `path` through either matcher, in
    /// declaration order (relative then absolute).
    pub fn watch_patterns_matching(&self, path: &str) -> Vec<String> {
        let mut patterns = self.watch_relative_patterns(path);
        patterns.extend(self.watch_absolute_patterns(path));
        patterns
    }

    pub fn ignore(&self, path: &str) -> bool {
        self.ignore_relative(path) || self.ignore_absolute(path)
    }

    pub fn ignore_relative(&self, path: &str) -> bool {
        !self.ignore_relative_patterns(path).is_empty()
    }

    /// Ignore patterns that match `path` through the relative matcher.
    pub fn ignore_relative_patterns(&self, path: &str) -> Vec<String> {
        let normalized_path = if path.starts_with('/') {
            path.to_owned()
        } else {
            format!("/{}", path)
        };

        self.ignore_patterns
            .iter()
            .filter(|pattern| !pattern.starts_with("/"))
            .filter(|ignore| {
                let normalized = ignore.trim_start_matches("./");
                anchored_pattern(&format!("/{}", normalized)).matches(&normalized_path)
            })
            .cloned()
            .collect()
    }

    pub fn ignore_absolute(&self, path: &str) -> bool {
        !self.ignore_absolute_patterns(path).is_empty()
    }

    /// Ignore patterns that match `path` through the absolute matcher.
    pub fn ignore_absolute_patterns(&self, path: &str) -> Vec<String> {
        self.ignore_patterns
            .iter()
            .filter(|pattern| pattern.starts_with("/"))
            .filter(|ignore| anchored_pattern(ignore).matches(path))
            .cloned()
            .collect()
    }

    /// Ignore patterns that match `path` through either matcher, in
    /// declaration order (relative then absolute).
    pub fn ignore_patterns_matching(&self, path: &str) -> Vec<String> {
        let mut patterns = self.ignore_relative_patterns(path);
        patterns.extend(self.ignore_absolute_patterns(path));
        patterns
    }

    pub fn commands(&self) -> Vec<String> {
        match &self.argv {
            // Display/wire form for ad-hoc exec rules: the exact argv joined
            // for presentation. Execution uses `command_lines()` so argv is
            // never re-parsed through a shell.
            Some(argv) => vec![argv.join(" ")],
            None => self.commands.clone(),
        }
    }

    /// Returns the exact argv for ad-hoc `exec` rules, or `None` for
    /// configured shell-command rules.
    pub fn argv(&self) -> Option<Vec<String>> {
        self.argv.clone()
    }

    /// Returns the commands this rule executes: configured shell commands, or
    /// the single exact argv for ad-hoc `exec` rules.
    pub fn command_lines(&self) -> Vec<CommandLine> {
        match &self.argv {
            Some(argv) => vec![CommandLine::Argv(argv.clone())],
            None => self
                .commands
                .iter()
                .map(|command| CommandLine::Shell(command.clone()))
                .collect(),
        }
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

        if self.command_lines().len() == 0 {
            return Err(format!(
                "job '{}' contains no command to run. Empty 'run' property.",
                name
            ));
        }

        if self.watch_patterns().len() == 0 && !self.run_on_init() {
            return Err(format!(
                "job '{}' must contain a `change` and/or `run_on_init` property.",
                name
            ));
        }

        for watch_pattern in self.watch_patterns() {
            match Pattern::new(&watch_pattern) {
                Ok(_) => (),
                Err(err) => {
                    return Err(vec![
                        format!(
                            "job '{}' contains an invalid `change` glob pattern '{}'.",
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
                            "job '{}' contains an invalid `ignore` glob pattern '{}'.",
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

/// One command selected for execution: either a configured shell command
/// string or an exact argv vector from ad-hoc `exec` mode.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum CommandLine {
    /// Configured task command, run through the user's shell (`$SHELL -c`).
    Shell(String),
    /// Ad-hoc `exec` command: program plus arguments, spawned directly
    /// without joining or re-parsing through a shell.
    Argv(Vec<String>),
}

impl CommandLine {
    /// Human-readable form used in logs, control payloads, and summaries.
    pub fn display(&self) -> String {
        match self {
            CommandLine::Shell(command) => command.clone(),
            CommandLine::Argv(argv) => argv.join(" "),
        }
    }
}

/// Flattens all rules into their execution command lines, preserving argv
/// for ad-hoc `exec` rules instead of joining and re-parsing them.
pub fn command_lines(rules: Vec<Rules>) -> Vec<CommandLine> {
    rules
        .iter()
        .flat_map(|rule| rule.command_lines())
        .collect::<Vec<CommandLine>>()
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

pub fn available_targets(rules: &[Rules]) -> String {
    // TASK-0077: the list names configured entries, so the vocabulary is
    // "jobs" at this boundary; runtime executions remain "tasks".
    let mut output = String::from("Available jobs\n");
    if rules.is_empty() {
        output.push_str("  (none)\n");
        return output;
    }

    for rule in rules {
        output.push_str(&format!("  - {}\n", rule.name));
        if !rule.watch_patterns.is_empty() {
            output.push_str(&format!("    change: {}\n", rule.watch_patterns.join(", ")));
        }
        if rule.run_on_init {
            output.push_str("    run_on_init: true\n");
        }
    }

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
    fn available_targets_lists_names_patterns_and_init_trigger() {
        let rules = vec![
            rule(
                "my tests @quick",
                &["cargo test"],
                &["tests/**", "src/**"],
                &[],
                false,
            ),
            rule("startup", &["echo ready"], &[], &[], true),
        ];

        assert_eq!(
            super::available_targets(&rules),
            "Available jobs\n  - my tests @quick\n    change: tests/**, src/**\n  - startup\n    run_on_init: true\n"
        );
    }

    #[test]
    fn available_targets_handles_empty_config() {
        assert_eq!(super::available_targets(&[]), "Available jobs\n  (none)\n");
    }

    #[test]
    fn argv_rule_keeps_exact_argv_and_never_splits_it() {
        let rule = Rules::from_argv(
            "unnamed".to_owned(),
            vec!["echo".to_owned(), "hello world".to_owned()],
            vec!["**/*.txt".to_owned()],
            vec![],
            true,
        );

        assert_eq!(
            rule.argv(),
            Some(vec!["echo".to_owned(), "hello world".to_owned()])
        );
        assert_eq!(
            rule.command_lines(),
            vec![super::CommandLine::Argv(vec![
                "echo".to_owned(),
                "hello world".to_owned()
            ])]
        );
        // Display form joins for presentation only; execution keeps argv.
        assert_eq!(rule.commands(), vec!["echo hello world"]);
    }

    #[test]
    fn shell_rule_keeps_each_command_as_shell_line() {
        let rule = rule("my tests", &["cargo test", "make lint"], &[], &[], false);

        assert_eq!(rule.argv(), None);
        assert_eq!(
            rule.command_lines(),
            vec![
                super::CommandLine::Shell("cargo test".to_owned()),
                super::CommandLine::Shell("make lint".to_owned()),
            ]
        );
    }

    #[test]
    fn command_lines_flatten_preserves_each_rules_boundary() {
        let rules = vec![
            rule("one", &["echo a"], &[], &[], false),
            Rules::from_argv(
                "two".to_owned(),
                vec!["printf".to_owned(), "%s".to_owned()],
                vec![],
                vec![],
                false,
            ),
        ];

        assert_eq!(
            super::command_lines(rules),
            vec![
                super::CommandLine::Shell("echo a".to_owned()),
                super::CommandLine::Argv(vec!["printf".to_owned(), "%s".to_owned()]),
            ]
        );
    }

    #[test]
    fn argv_rule_passes_validation_without_run_property() {
        let rule = Rules::from_argv(
            "unnamed".to_owned(),
            vec!["echo".to_owned()],
            vec!["**/*.txt".to_owned()],
            vec![],
            true,
        );
        assert!(rule.validate().is_ok());
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
            "job 'this is an invalid pattern' contains an invalid `change` glob pattern '**/foo_**.go'.
  Pattern syntax error near position 6: recursive wildcards must form a single path component
  Read more: https://en.wikipedia.org/wiki/Glob_(programming)"
        );

        let third_rule = &rules[2];
        assert!(third_rule.validate().is_err());
        assert_eq!(
            third_rule.validate().err().unwrap(),
            "job 'this is an invalid pattern 2' contains an invalid `ignore` glob pattern '**/**.*'.
  Pattern syntax error near position 5: recursive wildcards must form a single path component
  Read more: https://en.wikipedia.org/wiki/Glob_(programming)"
        );

        let fourth_rule = &rules[3];
        assert!(fourth_rule.validate().is_err());
        assert_eq!(
            fourth_rule.validate().err().unwrap(),
            "job 'missing trigger property' must contain a `change` and/or `run_on_init` property."
        );
    }
}
