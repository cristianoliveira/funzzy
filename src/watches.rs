use std::fmt;
use std::path::{Path, PathBuf};
use std::time::Duration;

use crate::plan::RunPlan;
use crate::rules::Rules;

/// Why a configured rule was selected or skipped for an explained path.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ExplainRule {
    pub name: String,
    /// Change patterns that matched the path.
    pub change_patterns: Vec<String>,
    /// Ignore patterns that matched and win over the change match.
    pub ignore_patterns: Vec<String>,
    /// Effective task cwd for diagnostics.
    pub cwd: String,
    /// Task-local environment names; values remain redacted.
    pub environment_keys: Vec<String>,
    /// Where the effective rule came from (TASK-0023): `task` or `group`
    /// when the responsible pattern was inherited from a group `on` section.
    pub origin: String,
}

/// Result of explaining a path against the configured rules.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct ExplainResult {
    /// Rules that would run for the path (change matched, no ignore).
    pub matched: Vec<ExplainRule>,
    /// Rules whose change pattern matched but an ignore pattern won.
    pub ignored: Vec<ExplainRule>,
    /// The filtered execution topology (stages + named group occurrences)
    /// after path filtering — the actual run plan preview, same planner as
    /// execution (TASK-0034). Empty when nothing matches.
    pub plan_stages: Vec<PlanStagePreview>,
}

/// One filtered execution stage for `explain` output (TASK-0034): a serial
/// task, or a named parallel group occurrence with its selected members.
/// Barriers and group names are shown without implying completion order.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum PlanStagePreview {
    Serial { task: String },
    Parallel { group: String, tasks: Vec<String> },
}

/// Execution facts relevant to an explained plan (TASK-0034): the effective
/// scheduler concurrency and the filesystem debounce window.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ExplainFacts {
    pub concurrency: usize,
    pub debounce: Duration,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum RunTargetError {
    Missing(String),
    Ambiguous {
        target: String,
        matches: Vec<String>,
    },
}

impl fmt::Display for RunTargetError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            RunTargetError::Missing(target) => {
                write!(formatter, "No target found for '{}'", target)
            }
            RunTargetError::Ambiguous { target, matches } => write!(
                formatter,
                "Target '{}' is ambiguous; matches: {}",
                target,
                matches.join(", ")
            ),
        }
    }
}

/// # Watches
///
/// Represents all rules in the yaml config loaded.
///
#[derive(Debug, Clone)]
pub struct Watches {
    rules: Vec<Rules>,
    topology: RunPlan,
    root: PathBuf,
    concurrency: usize,
    /// Debounce window per filesystem event batch (TASK-0031). Defaults to
    /// the historical one second unless `on.debounce` configures otherwise.
    debounce: Duration,
}
impl Watches {
    /// Convenience constructor resolving the workspace root from the process
    /// current directory. Keep usage at the outer boundary (composition root
    /// and tests); prefer [`Watches::with_root`] so core behavior does not
    /// depend on hidden process state.
    pub fn new(rules: Vec<Rules>) -> Self {
        let root = std::env::current_dir().expect("Unable to get current directory");
        Watches::with_root(rules, root)
    }

    /// Creates watches anchored at an explicit workspace root.
    pub fn with_root(rules: Vec<Rules>, root: PathBuf) -> Self {
        let concurrency = std::thread::available_parallelism()
            .map(|parallelism| parallelism.get())
            .unwrap_or(1);
        Watches::with_root_and_concurrency(rules, root, concurrency)
    }

    pub fn with_root_and_concurrency(rules: Vec<Rules>, root: PathBuf, concurrency: usize) -> Self {
        assert!(concurrency > 0, "watch concurrency must be positive");
        let topology = RunPlan::from_rules(rules.clone());
        Watches {
            rules,
            topology,
            root,
            concurrency,
            debounce: Duration::from_millis(1000),
        }
    }

    /// Overrides the filesystem debounce window (TASK-0031); the default is
    /// the historical one second.
    pub fn with_debounce(mut self, debounce: Duration) -> Self {
        self.debounce = debounce;
        self
    }

    /// The debounce window used per filesystem event batch.
    pub fn debounce(&self) -> Duration {
        self.debounce
    }

    /// Narrows visible rules while retaining barriers from original topology.
    pub fn select_target(&self, target: &str) -> Option<Self> {
        let rules: Vec<Rules> = self
            .rules
            .iter()
            .filter(|rule| rule.name.contains(target))
            .cloned()
            .collect();
        if rules.is_empty() {
            return None;
        }
        Some(Self {
            rules,
            topology: self
                .topology
                .clone()
                .filter(|rule| rule.name.contains(target)),
            root: self.root.clone(),
            concurrency: self.concurrency,
            debounce: self.debounce,
        })
    }

    /// The workspace root this watch planning is anchored to.
    pub fn root(&self) -> &Path {
        &self.root
    }

    /// Maximum simultaneously active tasks within one parallel group.
    pub fn concurrency(&self) -> usize {
        self.concurrency
    }

    fn normalize_paths<'a>(&'a self, path: &'a str) -> (PathBuf, Option<String>) {
        let provided_path = Path::new(path);
        let absolute_path = if provided_path.is_absolute() {
            provided_path.to_path_buf()
        } else {
            self.root.join(provided_path)
        };

        let relative_path = match absolute_path.strip_prefix(&self.root) {
            Ok(rel) => Some(format!("/{}", rel.display())),
            Err(_) => None,
        };

        (absolute_path, relative_path)
    }

    /// The root-relative form of a path used for rule matching (TASK-0023):
    /// the deterministic normalized path diagnostics report alongside the raw
    /// event path. Falls back to the provided path when it is outside the
    /// workspace root.
    pub fn normalized_path(&self, path: &str) -> String {
        let (_, relative) = self.normalize_paths(path);
        relative
            .map(|relative| relative.trim_start_matches('/').to_owned())
            .unwrap_or_else(|| path.to_owned())
    }

    /// Returns all configured targets.
    pub fn targets(&self) -> Vec<Rules> {
        self.rules.clone()
    }

    /// Returns rules whose names contain the requested target.
    pub fn target(&self, target: &str) -> Option<Vec<Rules>> {
        let rules = self
            .rules
            .iter()
            .filter(|rule| rule.name.contains(target))
            .cloned()
            .collect::<Vec<Rules>>();

        if rules.is_empty() {
            return None;
        }

        Some(rules)
    }

    /// Resolves a finite local-run target deterministically.
    ///
    /// An exact task name wins. `@tag` selectors intentionally run every
    /// match. Other substrings must identify one task; multiple matches are
    /// rejected rather than running an accidental superset in CI.
    pub fn run_target_plan(&self, target: &str) -> Result<RunPlan, RunTargetError> {
        let exact_matches = self
            .rules
            .iter()
            .filter(|rule| rule.name == target)
            .collect::<Vec<_>>();
        if exact_matches.len() > 1 {
            return Err(RunTargetError::Ambiguous {
                target: target.to_owned(),
                matches: exact_matches.iter().map(|rule| rule.name.clone()).collect(),
            });
        }
        if exact_matches.len() == 1 {
            return Ok(self.topology.clone().filter(|rule| rule.name == target));
        }

        let matches = self
            .rules
            .iter()
            .filter(|rule| rule.name.contains(target))
            .collect::<Vec<_>>();
        if matches.is_empty() {
            return Err(RunTargetError::Missing(target.to_owned()));
        }
        if matches.len() > 1 && !target.starts_with('@') {
            return Err(RunTargetError::Ambiguous {
                target: target.to_owned(),
                matches: matches.iter().map(|rule| rule.name.clone()).collect(),
            });
        }

        Ok(self
            .topology
            .clone()
            .filter(|rule| rule.name.contains(target)))
    }

    /// Selects a target without collapsing barriers around unmatched rules.
    pub fn target_plan(&self, target: &str) -> Option<RunPlan> {
        let plan = self
            .topology
            .clone()
            .filter(|rule| rule.name.contains(target));
        if plan.is_empty() {
            return None;
        }
        Some(plan)
    }

    /// Selects matching tasks while retaining original stage occurrences.
    pub fn watch_plan(&self, path: &str) -> Option<RunPlan> {
        let (absolute_path, relative_path) = self.normalize_paths(path);
        let absolute_path_str = absolute_path.to_str().unwrap_or_default();
        let plan = self.topology.clone().filter(|rule| {
            let ignored_by_absolute = rule.ignore_absolute(absolute_path_str);
            let ignored_by_relative = relative_path
                .as_ref()
                .map(|relative| rule.ignore_relative(relative))
                .unwrap_or(false);
            if ignored_by_absolute || ignored_by_relative {
                return false;
            }

            let watched_by_absolute = rule.watch_absolute(absolute_path_str);
            let watched_by_relative = relative_path
                .as_ref()
                .map(|relative| rule.watch_relative(relative))
                .unwrap_or(false);
            watched_by_absolute || watched_by_relative
        });
        if plan.is_empty() {
            return None;
        }
        Some(plan)
    }

    /// Routes one normalized event batch to zero or one generation (contract
    /// §1): scans the changed paths in deterministic order and returns the
    /// plan of the FIRST path that matches (change matched, not ignored),
    /// plus that trigger path for template expansion. A batch whose paths are
    /// all unmatched or ignored yields None.
    pub fn watch_plan_batch(&self, paths: &[String]) -> Option<(RunPlan, String)> {
        let mut sorted = paths.to_vec();
        sorted.sort();
        for path in sorted {
            if let Some(plan) = self.watch_plan(&path) {
                return Some((plan, path));
            }
        }
        None
    }

    /// Returns the commands for first rule found for the given path
    ///
    pub fn watch(&self, path: &str) -> Option<Vec<Rules>> {
        let (absolute_path, relative_path) = self.normalize_paths(path);
        let absolute_path_str = absolute_path.to_str().unwrap_or_default();

        let cmds = self
            .rules
            .iter()
            .cloned()
            .filter(|r| {
                let ignored_by_absolute = r.ignore_absolute(absolute_path_str);
                let ignored_by_relative = relative_path
                    .as_ref()
                    .map(|rel| r.ignore_relative(rel))
                    .unwrap_or(false);

                if ignored_by_absolute || ignored_by_relative {
                    return false;
                }

                let watched_by_absolute = r.watch_absolute(absolute_path_str);
                let watched_by_relative = relative_path
                    .as_ref()
                    .map(|rel| r.watch_relative(rel))
                    .unwrap_or(false);

                watched_by_absolute || watched_by_relative
            })
            .collect::<Vec<Rules>>();

        if !cmds.is_empty() {
            Some(cmds)
        } else {
            None
        }
    }

    /// Selects init tasks while retaining original stage occurrences.
    pub fn run_on_init_plan(&self) -> Option<RunPlan> {
        let plan = self.topology.clone().filter(Rules::run_on_init);
        if plan.is_empty() {
            return None;
        }
        Some(plan)
    }

    /// Returns the commands for the rules that should run on init
    ///
    pub fn run_on_init(&self) -> Option<Vec<Rules>> {
        let cmds = self
            .rules
            .iter()
            .cloned()
            .filter(|r| r.run_on_init())
            .collect::<Vec<Rules>>();

        if !cmds.is_empty() {
            Some(cmds)
        } else {
            None
        }
    }

    /// Explains which configured rules match, ignore, or miss `path`.
    ///
    /// Reuses the exact matching policy of [`Watches::watch`] (same path
    /// normalization and rule matchers); this method only reports *which*
    /// change and ignore patterns matched per rule. It never starts a watcher
    /// or executes a task.
    pub fn explain(&self, path: &str) -> ExplainResult {
        let (absolute_path, relative_path) = self.normalize_paths(path);
        let absolute_path_str = absolute_path.to_str().unwrap_or_default();

        let mut matched = vec![];
        let mut ignored = vec![];

        for rule in &self.rules {
            // Mirror Watches::watch exactly: absolute patterns match the
            // absolute path; relative patterns match the root-relative path.
            let mut change_patterns = rule.watch_absolute_patterns(absolute_path_str);
            if let Some(rel) = relative_path.as_ref() {
                change_patterns.extend(rule.watch_relative_patterns(rel));
            }
            if change_patterns.is_empty() {
                continue;
            }

            let mut ignore_patterns = rule.ignore_absolute_patterns(absolute_path_str);
            if let Some(rel) = relative_path.as_ref() {
                ignore_patterns.extend(rule.ignore_relative_patterns(rel));
            }
            let cwd = rule
                .cwd()
                .map(|cwd| self.root.join(cwd))
                .unwrap_or_else(|| self.root.clone())
                .display()
                .to_string();
            let environment_keys = rule.environment().keys().cloned().collect();
            if ignore_patterns.is_empty() {
                let origin = Self::origin_for(rule, change_patterns.first(), None);
                matched.push(ExplainRule {
                    name: rule.name.clone(),
                    change_patterns,
                    ignore_patterns: vec![],
                    cwd,
                    environment_keys,
                    origin,
                });
            } else {
                let origin = Self::origin_for(rule, None, ignore_patterns.first());
                ignored.push(ExplainRule {
                    name: rule.name.clone(),
                    change_patterns,
                    ignore_patterns,
                    cwd,
                    environment_keys,
                    origin,
                });
            }
        }

        ExplainResult {
            matched,
            ignored,
            // TASK-0034: the filtered execution topology uses the exact same
            // planner as execution (watch_plan), so the preview can never
            // drift from what would actually run.
            plan_stages: self
                .watch_plan(path)
                .map(|plan| {
                    plan.stages
                        .into_iter()
                        .map(|stage| match stage {
                            crate::plan::Stage::Serial(task) => {
                                PlanStagePreview::Serial { task: task.name }
                            }
                            crate::plan::Stage::Parallel { group, tasks } => {
                                // The stage carries the bare group name; the
                                // occurrence identity lives on the member
                                // tasks (e.g. `checks#2`), so previews show
                                // the same occurrence label as execution.
                                let occurrence = tasks
                                    .first()
                                    .and_then(|t| t.group_occurrence.clone())
                                    .unwrap_or_else(|| group.clone());
                                PlanStagePreview::Parallel {
                                    group: occurrence,
                                    tasks: tasks.into_iter().map(|t| t.name).collect(),
                                }
                            }
                        })
                        .collect()
                })
                .unwrap_or_default(),
        }
    }

    /// The effective-rule origin for one rule decision: `group` when the
    /// responsible pattern was inherited from a group `on` section, else
    /// `task`. The responsible pattern is the first matching change pattern
    /// (or the first matching ignore pattern when the ignore won).
    fn origin_for(rule: &Rules, change: Option<&String>, ignore: Option<&String>) -> String {
        let inherited = rule.inherited_patterns();
        let responsible = change.or(ignore);
        match responsible {
            Some(pattern) if inherited.iter().any(|p| p == pattern) => "group".to_owned(),
            _ => "task".to_owned(),
        }
    }

    /// Extract the directory to watch from a glob pattern.
    /// For example:
    /// - "src/**" -> "src"
    /// - "/tmp/**" -> "/tmp"
    /// - "examples/workdir/**/*" -> "examples/workdir"
    fn extract_watch_directory(pattern: &str, current_dir: &std::path::Path) -> String {
        let absolute_pattern = if pattern.starts_with("/") {
            pattern.to_string()
        } else {
            let mut abs = current_dir.to_path_buf();
            abs.push(pattern);
            abs.to_str().unwrap().to_string()
        };

        // Split by '/' and collect segments until we hit a glob metacharacter
        let mut segments = Vec::new();
        let is_absolute = absolute_pattern.starts_with('/');
        for segment in absolute_pattern.split('/') {
            if segment.contains('*')
                || segment.contains('?')
                || segment.contains('[')
                || segment.contains('{')
            {
                break;
            }
            if !segment.is_empty() {
                segments.push(segment);
            }
        }

        if segments.is_empty() {
            return current_dir.to_str().unwrap().to_string();
        }

        let mut result = String::new();
        if is_absolute {
            result.push('/');
        }
        result.push_str(&segments.join("/"));
        result
    }

    /// Returns the list of rules that contains absolute path
    ///
    pub fn paths_to_watch(&self) -> Option<Vec<String>> {
        let mut paths = Vec::new();

        for rule in &self.rules {
            for pattern in rule.watch_patterns() {
                let dir = Self::extract_watch_directory(&pattern, &self.root);
                if !paths.contains(&dir) {
                    paths.push(dir);
                }
            }
        }

        // Always watch current directory as fallback
        let current_dir_str = self.root.to_str().unwrap().to_string();
        if !paths.contains(&current_dir_str) {
            paths.push(current_dir_str);
        }

        if !paths.is_empty() {
            Some(paths)
        } else {
            None
        }
    }
}

#[cfg(test)]
mod tests {
    extern crate glob;
    extern crate notify;
    extern crate yaml_rust;

    use super::*;
    use crate::config;
    use crate::plan::Stage;
    use crate::rules;
    use std::env;

    fn get_absolute_path(path: &str) -> String {
        let mut absolute_path = env::current_dir().unwrap();
        absolute_path.push(path);
        absolute_path.to_str().unwrap().to_string()
    }

    #[test]
    fn it_loads_from_args() {
        let watches = Watches::new(
            config::from_argv(vec![".".to_owned()], vec!["cargo build".to_owned()])
                .expect("Error while parsing rules from string"),
        );

        assert!(watches.watch(&get_absolute_path("src/main.rs")).is_some());
        assert!(watches.watch(&get_absolute_path("test/main.rs")).is_some());
        assert!(watches.watch(&get_absolute_path(".")).is_some());

        let result = rules::commands(watches.watch(&get_absolute_path(".")).unwrap());
        assert_eq!(vec!["cargo build"], result);
    }

    #[test]
    fn it_watches_test_path() {
        let file_content = "
        - name: my tests
          run: 'cargo tests'
          change: 'tests/**'
        ";
        let watches = Watches::new(config::from_yaml(&file_content).expect("Error parsing yaml"));
        assert!(watches.watch(&get_absolute_path("tests/test.rs")).is_some());
        assert!(watches.watch("tests/tests.rs").is_some());
        assert!(watches.watch("tests/ruby.rb").is_some());
        assert!(watches.watch("tests/folder/other.rs").is_some())
    }

    #[test]
    fn it_watches_specific_path() {
        let file_content = "
        - name: my tests
          run: 'cargo tests'
          change: './tests/foo/bar.rs'
        ";
        let watches = Watches::new(config::from_yaml(&file_content).expect("Error parsing yaml"));
        assert!(watches.watch("./tests/foo/bar.rs").is_some())
    }

    #[test]
    fn it_anchors_relative_patterns_to_root() {
        let file_content = "
        - name: txt files
          run: 'echo txt'
          change: 'src/*.txt'
        ";
        let watches = Watches::new(config::from_yaml(&file_content).expect("Error parsing yaml"));

        let root = std::env::current_dir().unwrap();
        let inside = root.join("src/foo.txt");
        let outside = root.join(".tmp/src/foo.txt");

        assert!(watches.watch(inside.to_str().unwrap()).is_some());
        assert!(watches.watch(outside.to_str().unwrap()).is_none());
    }

    #[test]
    fn it_explains_matched_ignored_and_unmatched_paths() {
        let file_content = "
        - name: my tests
          cwd: crates/tests
          env: { TOKEN: secret }
          run: 'cargo tests'
          change: 'tests/**'
          ignore: 'tests/ignored/**'

        - name: docs
          run: 'echo docs'
          change: 'docs/**'
        ";
        let watches = Watches::new(config::from_yaml(&file_content).expect("Error parsing yaml"));

        let matched = watches.explain(&get_absolute_path("tests/foo.rs"));
        assert_eq!(matched.matched.len(), 1, "one rule matches tests/**");
        assert_eq!(matched.matched[0].name, "my tests");
        assert_eq!(matched.matched[0].change_patterns, vec!["tests/**"]);
        assert!(matched.matched[0].ignore_patterns.is_empty());
        assert!(matched.matched[0].cwd.ends_with("crates/tests"));
        assert_eq!(matched.matched[0].environment_keys, vec!["TOKEN"]);
        assert_eq!(matched.matched[0].origin, "task");
        assert!(!format!("{:?}", matched.matched[0]).contains("secret"));
        assert!(matched.ignored.is_empty());

        let ignored = watches.explain(&get_absolute_path("tests/ignored/foo.rs"));
        assert!(ignored.matched.is_empty());
        assert_eq!(ignored.ignored.len(), 1, "ignore wins over change match");
        assert_eq!(ignored.ignored[0].name, "my tests");
        assert_eq!(ignored.ignored[0].ignore_patterns, vec!["tests/ignored/**"]);
        assert_eq!(ignored.ignored[0].change_patterns, vec!["tests/**"]);
        assert_eq!(ignored.ignored[0].origin, "task");

        let unmatched = watches.explain(&get_absolute_path("unknown/path.rs"));
        assert!(unmatched.matched.is_empty());
        assert!(unmatched.ignored.is_empty());
    }

    #[test]
    fn it_explains_relative_paths_the_same_as_absolute() {
        let file_content = "
        - name: my tests
          run: 'cargo tests'
          change: 'tests/**'
        ";
        let watches = Watches::new(config::from_yaml(&file_content).expect("Error parsing yaml"));

        let absolute = watches.explain(&get_absolute_path("tests/foo.rs"));
        let relative = watches.explain("tests/foo.rs");

        assert_eq!(absolute.matched.len(), 1);
        assert_eq!(
            relative.matched[0].change_patterns,
            absolute.matched[0].change_patterns
        );
    }

    #[test]
    fn it_explains_absolute_pattern_paths() {
        let file_content = "
        - name: tmp watcher
          run: 'echo tmp'
          change: '/tmp/funzzy-explain-*/*.txt'
        ";
        let watches = Watches::new(config::from_yaml(&file_content).expect("Error parsing yaml"));

        let matched = watches.explain("/tmp/funzzy-explain-1/foo.txt");
        assert_eq!(matched.matched.len(), 1);
        assert_eq!(matched.matched[0].name, "tmp watcher");
        assert_eq!(
            matched.matched[0].change_patterns,
            vec!["/tmp/funzzy-explain-*/*.txt"]
        );
    }

    #[test]
    fn it_explains_merged_group_rules_from_nested_groups() {
        let content = std::fs::read_to_string("examples/nested-groups.yml").expect("read example");
        let watches = Watches::new(config::from_yaml(&content).expect("Error parsing yaml"));

        // frontend-build inherits the group change patterns; the effective
        // rule origin must be reported as the group, not the task.
        let matched = watches.explain("src/frontend/App.tsx");
        let frontend = matched
            .matched
            .iter()
            .find(|rule| rule.name == "frontend-build")
            .expect("frontend-build must match");
        assert_eq!(
            frontend.origin, "group",
            "group-inherited patterns must be reported with group origin"
        );
        let names: Vec<&str> = matched
            .matched
            .iter()
            .map(|rule| rule.name.as_str())
            .collect();
        assert!(
            names.contains(&"frontend-build"),
            "group-merged rule must match: {:?}",
            names
        );

        // Group ignore wins over the inherited change match.
        let ignored = watches.explain("src/frontend/server.log");
        let ignored_names: Vec<&str> = ignored
            .ignored
            .iter()
            .map(|rule| rule.name.as_str())
            .collect();
        assert!(
            ignored_names.contains(&"frontend-build"),
            "group ignore must win: {:?}",
            ignored_names
        );
    }

    #[test]
    fn it_doesnot_watch_test_path() {
        let file_content = "
        - name: my source
          run: 'cargo build'
          change: 'src/**'
        ";
        let watches = Watches::new(config::from_yaml(&file_content).expect("Error parsing yaml"));

        assert!(watches.watch(&get_absolute_path("events.yaml")).is_none());
        assert!(watches.watch("tests/").is_none());
        assert!(watches.watch("tests/test.rs").is_none());
        assert!(watches.watch("tests/folder/other.rs").is_none());
    }

    #[test]
    fn it_creates_a_list_of_shell_commands() {
        let file_content = "
        - name: my source
          run: 'cargo build'
          change: 'src/**'
        ";
        let watches = Watches::new(config::from_yaml(&file_content).expect("Error parsing yaml"));
        let result = rules::commands(watches.watch("src/test.rs").unwrap());
        assert_eq!("cargo build", result[0])
    }

    #[test]
    fn it_works_with_multiples_items() {
        let file_content = "
        - name: my source
          run: 'cargo build'
          change: 'src/**'

        - name: other
          run: 'cargo test'
          change: 'test/**'
        ";
        let watches = Watches::new(config::from_yaml(&file_content).expect("Error parsing yaml"));

        let result = rules::commands(watches.watch("test/test.rs").unwrap());
        assert_eq!("cargo test", result[0]);

        let result_src = rules::commands(watches.watch("src/test.rs").unwrap());
        assert_eq!("cargo build", result_src[0]);
    }

    #[test]
    fn it_allows_many_rules_watching_same_path() {
        let file_content = "
        - name: same path
          run: 'echo same'
          change: '**'

        - name: my source
          run: 'cargo build'
          change: 'src/**'

        - name: other
          run: 'cargo test'
          change: 'test/**'
        ";
        let watches = Watches::new(config::from_yaml(&file_content).expect("Error parsing yaml"));

        let result = rules::commands(watches.watch("src/test.rs").unwrap());
        assert_eq!(vec!["echo same", "cargo build"], result);

        let result_multiple = rules::commands(watches.watch("test/test.rs").unwrap());
        assert_eq!(vec!["echo same", "cargo test"], result_multiple);
    }

    #[test]
    fn it_ignores_pattern() {
        let file_content = "
        - name: my source
          run: 'cargo build'
          change: 'src/**'
          ignore: 'src/test/**'
        ";
        let watches = Watches::new(config::from_yaml(&file_content).expect("Error parsing yaml"));
        assert!(watches.watch("src/other.rb").is_some());
        assert!(watches.watch("src/test.txt").is_some());
        assert!(watches.watch("src/test/other.tmp").is_none())
    }

    #[test]
    fn it_ignores_a_list_of_patterns() {
        let file_content = "
        - name: my source
          run: 'cargo build'
          change: 'src/**'
          ignore: ['src/test/**', 'src/tmp/**']
        ";
        let watches = Watches::new(config::from_yaml(&file_content).expect("Error parsing yaml"));
        assert!(watches.watch("src/other.rb").is_some());
        assert!(watches.watch("src/test.txt").is_some());
        assert!(watches.watch("src/tmp/test.txt").is_none());
        assert!(watches.watch("src/test/other.tmp").is_none())
    }

    #[test]
    fn it_ignores_nested_tooling_cache_dirs_and_the_directory_event() {
        // Tooling may emit an event for either the cache directory or a file in it.
        // Both patterns are needed because `**/.pi/**` does not match `tests/.pi`.
        let file_content = "
            - name: lint
              run: 'cargo fmt'
              change: 'tests/**'
              ignore: ['**/.pi', '**/.pi/**']
        ";
        let watches = Watches::new(config::from_yaml(&file_content).expect("Error parsing yaml"));
        assert!(watches.watch("tests/foo.rs").is_some());
        assert!(watches.watch("tests/.pi").is_none());
        assert!(watches.watch("tests/.pi/ast-index.sqlite").is_none());
    }

    #[test]
    fn it_ignores_tooling_cache_in_real_watch_config() {
        let here = std::path::Path::new(env!("CARGO_MANIFEST_DIR"));
        let config =
            std::fs::read_to_string(here.join(".watch.yaml")).expect("missing .watch.yaml");
        let watches = Watches::new(config::from_yaml(&config).expect("Error parsing yaml"));

        for path in ["tests/.pi", "tests/.pi/ast-index.sqlite"] {
            let matched: Vec<String> = watches
                .watch(path)
                .map(|rules| rules.into_iter().map(|r| r.name).collect())
                .unwrap_or_default();
            assert!(
                matched.is_empty(),
                "tooling cache path '{}' must not trigger any task, got: {:?}",
                path,
                matched
            );
        }
    }

    #[test]
    fn it_returns_on_init_rules() {
        let file_content = "
            - name: my source
              run: 'cargo build'
              change: 'src/**'
              run_on_init: true

            - name: my source
              run: ['cat foo', 'cat bar']
              change: 'src/**'
              run_on_init: true

            - name: other
              run: 'cargo test'
              change: 'test/**'
            ";
        let watches = Watches::new(config::from_yaml(&file_content).expect("Error parsing yaml"));
        let results = rules::commands(watches.run_on_init().unwrap());

        assert_eq!(
            results,
            vec![
                "cargo build".to_string(),
                "cat foo".to_string(),
                "cat bar".to_string(),
            ]
        );
    }

    #[test]
    fn it_returns_rules_with_absolute_path_and_current_dir() {
        let file_content = "
            - name: my source
              run: ['cat foo', 'cat bar']
              change: 'src/**'

            - name: rule with absolute path
              run: 'cargo build'
              change: 
                - 'src/**'
                - '/tmp/**'
                - '/User/**'

            - name: it does not consider the ignored rules
              run: 'cargo test'
              change: 'test/**'
              ignored: '/test/**'

            - name: another with absolute path
              run: echo 'absolute paths'
              change: 
                - '/dev/**'
                - '/usr/**'
                - '/etc/**'
            ";
        let watches = Watches::new(config::from_yaml(&file_content).expect("Error parsing yaml"));
        let results = watches.paths_to_watch().expect("No rules found");

        let current_dir = std::env::current_dir().expect("Unable to get current directory");
        // Compute expected directories: all patterns converted to directories, plus current_dir
        let mut expected = Vec::new();
        let patterns = vec![
            "src/**", "src/**", "/tmp/**", "/User/**", "test/**", "/dev/**", "/usr/**", "/etc/**",
        ];
        for pattern in patterns {
            let dir = Watches::extract_watch_directory(pattern, &current_dir);
            if !expected.contains(&dir) {
                expected.push(dir);
            }
        }
        // Add current directory if not already present (it should be added by paths_to_watch)
        let current_dir_str = current_dir.to_str().unwrap().to_string();
        if !expected.contains(&current_dir_str) {
            expected.push(current_dir_str);
        }

        assert_eq!(results.len(), expected.len());
        // Order should match iteration order
        for (i, expected_dir) in expected.iter().enumerate() {
            assert_eq!(&results[i], expected_dir);
        }
    }

    #[test]
    fn it_uses_injected_root_with_spaces_for_relative_patterns() {
        let file_content = "
        - name: txt files
          run: 'echo txt'
          change: 'src/*.txt'
    ";
        let rules = config::from_yaml(&file_content).expect("Error parsing yaml");
        let root =
            std::env::temp_dir().join(format!("funzzy root with spaces {}", std::process::id()));
        let watches = Watches::with_root(rules, root.clone());

        let inside = root.join("src/foo.txt");
        let outside = std::env::temp_dir()
            .join(format!("funzzy elsewhere {}", std::process::id()))
            .join("src/foo.txt");

        assert!(
            watches.watch(inside.to_str().unwrap()).is_some(),
            "relative patterns must anchor to the injected root"
        );
        assert!(
            watches.watch(outside.to_str().unwrap()).is_none(),
            "paths outside the injected root must not match relative patterns"
        );
    }

    #[test]
    fn it_matches_absolute_patterns_outside_the_injected_root() {
        let file_content = "
        - name: outside
          run: 'echo outside'
          change: '/tmp/**'
    ";
        let rules = config::from_yaml(&file_content).expect("Error parsing yaml");
        let root = std::env::temp_dir().join(format!("funzzy-root-{}", std::process::id()));
        let watches = Watches::with_root(rules, root);

        let outside = std::path::Path::new("/tmp/funzzy-outside-marker/foo.txt");
        assert!(
            watches.watch(outside.to_str().unwrap()).is_some(),
            "absolute patterns must still match outside the injected root"
        );
    }

    #[test]
    fn local_run_target_prefers_exact_allows_tags_and_rejects_ambiguity() {
        let rules = ["build", "build docs", "lint @quick", "test @quick"]
            .iter()
            .map(|name| {
                Rules::new(
                    (*name).to_owned(),
                    vec!["true".to_owned()],
                    vec!["src/**".to_owned()],
                    vec![],
                    false,
                )
            })
            .collect();
        let watches = Watches::new(rules);

        assert_eq!(
            watches
                .run_target_plan("build")
                .expect("exact target")
                .task_names(),
            vec!["build"]
        );
        assert_eq!(
            watches
                .run_target_plan("@quick")
                .expect("tag target")
                .task_names(),
            vec!["lint @quick", "test @quick"]
        );
        assert!(matches!(
            watches.run_target_plan("quick"),
            Err(RunTargetError::Ambiguous { .. })
        ));
        assert_eq!(
            watches.run_target_plan("missing"),
            Err(RunTargetError::Missing("missing".to_owned()))
        );
    }

    #[test]
    fn target_selection_keeps_separated_group_occurrences_and_concurrency() {
        let rules = vec![
            Rules::new(
                "A selected".to_owned(),
                vec!["echo a".to_owned()],
                vec!["src/**".to_owned()],
                vec![],
                true,
            )
            .with_parallel("checks".to_owned()),
            Rules::new(
                "separator".to_owned(),
                vec!["echo separator".to_owned()],
                vec!["other/**".to_owned()],
                vec![],
                false,
            ),
            Rules::new(
                "C selected".to_owned(),
                vec!["echo c".to_owned()],
                vec!["src/**".to_owned()],
                vec![],
                true,
            )
            .with_parallel("checks".to_owned()),
        ];
        let watches = Watches::with_root_and_concurrency(rules, env::current_dir().unwrap(), 2)
            .select_target("selected")
            .expect("selected targets");
        let plan = watches.target_plan("selected").expect("target plan");

        assert_eq!(watches.concurrency(), 2);
        assert_eq!(plan.stages.len(), 2);
        assert!(plan
            .stages
            .iter()
            .all(|stage| matches!(stage, Stage::Parallel { tasks, .. } if tasks.len() == 1)));
    }

    #[test]
    fn one_batch_maps_to_one_generation_via_first_deterministic_match() {
        let rules = vec![
            Rules::new(
                "build".to_owned(),
                vec!["echo build".to_owned()],
                vec!["src/**".to_owned()],
                vec![],
                false,
            ),
            Rules::new(
                "docs".to_owned(),
                vec!["echo docs".to_owned()],
                vec!["docs/**".to_owned()],
                vec![],
                false,
            ),
        ];
        let watches = Watches::with_root_and_concurrency(rules, env::current_dir().unwrap(), 1);

        // Deterministic order: docs/** sorts before src/**, so "docs/a.md" is
        // the trigger even though "src/main.rs" also matches.
        let (plan, trigger) = watches
            .watch_plan_batch(&["src/main.rs".to_owned(), "docs/a.md".to_owned()])
            .expect("batch matches");
        assert_eq!(trigger, "docs/a.md");
        assert_eq!(plan.task_names(), vec!["docs".to_owned()]);
    }

    #[test]
    fn batch_with_only_ignored_paths_yields_no_generation() {
        let rules = vec![Rules::new(
            "build".to_owned(),
            vec!["echo build".to_owned()],
            vec!["src/**".to_owned()],
            vec!["src/generated/**".to_owned()],
            false,
        )];
        let watches = Watches::with_root_and_concurrency(rules, env::current_dir().unwrap(), 1);

        assert!(
            watches
                .watch_plan_batch(&["src/generated/out.rs".to_owned()])
                .is_none(),
            "ignored-only batch must not schedule"
        );
    }

    #[test]
    fn batch_with_ignored_and_matching_paths_uses_the_matching_one() {
        let rules = vec![Rules::new(
            "build".to_owned(),
            vec!["echo build".to_owned()],
            vec!["src/**".to_owned()],
            vec!["src/generated/**".to_owned()],
            false,
        )];
        let watches = Watches::with_root_and_concurrency(rules, env::current_dir().unwrap(), 1);

        let (plan, trigger) = watches
            .watch_plan_batch(&["src/generated/out.rs".to_owned(), "src/main.rs".to_owned()])
            .expect("matching path must win");
        assert_eq!(trigger, "src/main.rs");
        assert_eq!(plan.task_names(), vec!["build".to_owned()]);
    }

    #[test]
    fn empty_batch_never_schedules() {
        let rules = vec![Rules::new(
            "build".to_owned(),
            vec!["echo build".to_owned()],
            vec!["src/**".to_owned()],
            vec![],
            false,
        )];
        let watches = Watches::with_root_and_concurrency(rules, env::current_dir().unwrap(), 1);
        assert!(watches.watch_plan_batch(&[]).is_none());
    }

    #[test]
    fn it_returns_an_error_when_fail_to_load_config_file() {
        // assert!(config::from_yaml(
        //     &r#"
        // - name: run tests
        //   run: [
        //     "yarn test {{filepath}}",
        //     "echo '{{filepath}}' | sed -r 's\/.tsx/\/'"
        //   ]
        //   change: 'src/**'
        // "#
        // )
        // .is_err());

        assert!(config::from_yaml(
            &r#"
        - name: run tests
          run: [
            "yarn test {{filepath}}", 
          change: 'src/**'
        "#
        )
        .is_err());

        assert!(config::from_yaml(
            &r#"
        - name: other
          run: 'cargo test'
          change: 'test/**'
        "#
        )
        .is_ok());
    }
}

#[cfg(test)]
mod explain_plan_tests {
    use super::*;

    fn config(content: &str) -> Vec<Rules> {
        crate::config::from_yaml(content).expect("parse config")
    }

    #[test]
    fn explain_plan_shows_serial_stages_in_order() {
        let watches = Watches::new(config(
            "jobs:\n  - name: a\n    run: echo a\n    change: 'src/**'\n  - name: b\n    run: echo b\n    change: 'src/**'\n",
        ));
        let result = watches.explain("src/x.rs");
        assert_eq!(result.matched.len(), 2);
        assert_eq!(
            result.plan_stages,
            vec![
                PlanStagePreview::Serial {
                    task: "a".to_owned()
                },
                PlanStagePreview::Serial {
                    task: "b".to_owned()
                },
            ]
        );
    }

    #[test]
    fn explain_plan_shows_parallel_group_occurrence() {
        let watches = Watches::new(config(
            "jobs:\n  - name: a\n    parallel: checks\n    run: echo a\n    change: 'src/**'\n  - name: b\n    parallel: checks\n    run: echo b\n    change: 'src/**'\n",
        ));
        let result = watches.explain("src/x.rs");
        assert_eq!(
            result.plan_stages,
            vec![PlanStagePreview::Parallel {
                group: "checks#1".to_owned(),
                tasks: vec!["a".to_owned(), "b".to_owned()],
            }]
        );
    }

    #[test]
    fn explain_plan_keeps_separated_group_occurrences() {
        let watches = Watches::new(config(
            "jobs:\n  - name: a\n    parallel: x\n    run: echo a\n    change: 'src/**'\n  - name: sep\n    run: echo sep\n    change: 'src/**'\n  - name: c\n    parallel: x\n    run: echo c\n    change: 'src/**'\n",
        ));
        let result = watches.explain("src/x.rs");
        assert_eq!(
            result.plan_stages,
            vec![
                PlanStagePreview::Parallel {
                    group: "x#1".to_owned(),
                    tasks: vec!["a".to_owned()],
                },
                PlanStagePreview::Serial {
                    task: "sep".to_owned()
                },
                PlanStagePreview::Parallel {
                    group: "x#2".to_owned(),
                    tasks: vec!["c".to_owned()],
                },
            ]
        );
    }

    #[test]
    fn explain_plan_excludes_ignored_and_unmatched_tasks() {
        let watches = Watches::new(config(
            "jobs:\n  - name: hit\n    run: echo hit\n    change: 'src/**'\n  - name: ignored\n    run: echo ignored\n    change: 'src/**'\n    ignore: 'src/**'\n  - name: other\n    run: echo other\n    change: 'docs/**'\n",
        ));
        let result = watches.explain("src/x.rs");
        assert_eq!(result.matched.len(), 1);
        assert_eq!(result.ignored.len(), 1);
        assert_eq!(
            result.plan_stages,
            vec![PlanStagePreview::Serial {
                task: "hit".to_owned()
            }]
        );
    }

    #[test]
    fn explain_plan_is_empty_for_unmatched_path() {
        let watches = Watches::new(config(
            "jobs:\n  - name: a\n    run: echo a\n    change: 'src/**'\n",
        ));
        let result = watches.explain("docs/x.md");
        assert!(result.matched.is_empty());
        assert!(result.plan_stages.is_empty());
    }
}
