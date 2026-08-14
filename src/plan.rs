//! Execution domain: task-aware run plans and outcomes (TASK-0024/0025).
//!
//! `RunPlan` preserves workflow topology as ordered serial tasks and named
//! parallel-group occurrences with barriers, instead of a flat command list.
//! `TaskPlan` keeps stable task identity, sequential command order, and
//! expanded path values. `RunOutcome`/`TaskOutcome` combine results
//! order-independently, keyed by task identity.
//!
//! This module is pure: planning, filtering, and outcome combination have no
//! process, stdout, control-socket, or threading side effects.

use crate::rules::{CommandLine, Rules};
use crate::template::{self, TemplateOptions};
use sha2::{Digest, Sha256};
use std::collections::BTreeMap;
use std::path::{Component, Path, PathBuf};

/// Schema version of the canonical signature encoding (contract §5). Bump
/// only on a breaking encoding change; bumping invalidates all old profiles.
pub const SIGNATURE_SCHEMA_VERSION: u64 = 1;

/// Process context applied only to one task's child commands.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct TaskContext {
    pub cwd: Option<PathBuf>,
    pub environment: BTreeMap<String, String>,
}

/// Stable execution identity (contract §5): canonical SHA-256 over the run
/// plan, topology, cwd, declared environment content, jobs, fail-fast, and
/// schema version. Never displays command or environment content; safe to
/// persist as a profile key.
#[derive(Clone, Debug, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct ExecutionSignature(pub String);

impl std::fmt::Display for ExecutionSignature {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter.write_str(&self.0)
    }
}

/// One task selected for execution, with stable identity and sequential
/// command order preserved.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TaskPlan {
    /// Stable task identity: the configured name.
    pub name: String,
    /// Position in the parsed workflow (stable within one plan).
    pub position: usize,
    /// Sequential commands, in declared order, after template expansion.
    pub commands: Vec<CommandLine>,
    /// Optional named `parallel` group this task belongs to.
    pub parallel: Option<String>,
    /// Stable group-occurrence identity `name#N` (contract §1): the named
    /// parallel group plus its contiguous occurrence index within the plan.
    /// None for serial tasks. Stable from plan build through serialization.
    pub group_occurrence: Option<String>,
    /// The original rule, kept for matching/presentation consumers.
    pub rule: Rules,
    /// Effective child process context. Relative cwd is resolved before spawn.
    pub context: TaskContext,
}

/// One stage of a run: a serial task or a named parallel-group occurrence.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Stage {
    /// A task that runs alone between barriers.
    Serial(TaskPlan),
    /// A contiguous occurrence of tasks sharing one `parallel` group name.
    Parallel { group: String, tasks: Vec<TaskPlan> },
}

/// Ordered execution plan preserving barriers and group occurrences.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct RunPlan {
    pub stages: Vec<Stage>,
}

/// Per-task outcome, order-independent combination keyed by task identity.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum TaskOutcome {
    Passed,
    Failed { failures: Vec<String> },
    Cancelled,
    Skipped,
}

/// Overall run outcome derived deterministically from task outcomes.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct RunOutcome {
    /// Task identity (+ optional parallel group) -> outcome, in plan order.
    /// Ordering inside a named parallel group is explicitly unspecified;
    /// combination is keyed by task identity (TASK-0028).
    pub tasks: Vec<(String, Option<String>, TaskOutcome)>,
}

impl TaskPlan {
    /// Expands template variables in this task's commands for the given
    /// path options, returning the expanded commands and any unknown
    /// variables. Pure: no process, stdout, or control-socket side effects.
    pub fn expand(&self, opts: &TemplateOptions) -> (Vec<CommandLine>, Vec<String>) {
        let mut unknown = vec![];
        let mut task_options = opts.clone();
        if let Some(cwd) = &self.context.cwd {
            task_options.current_dir = cwd.display().to_string();
        }
        let expanded = self
            .commands
            .iter()
            .map(|cmd| {
                let out = template::template_line(cmd.clone(), task_options.clone());
                unknown.extend(out.unknown_variables);
                out.command
            })
            .collect();
        (expanded, unknown)
    }
}

impl RunPlan {
    /// Resolves every task cwd from injected workspace root. Absolute paths
    /// and any `..` component are rejected; tasks cannot escape workspace.
    /// Existence is validated by executor immediately before first spawn.
    pub fn resolve_context(&self, workspace_root: &Path) -> Result<RunPlan, String> {
        let resolve_task = |task: &TaskPlan| -> Result<TaskPlan, String> {
            let mut resolved = task.clone();
            let cwd = match &task.context.cwd {
                None => workspace_root.to_path_buf(),
                Some(cwd) if cwd.is_absolute() => {
                    return Err(format!(
                        "Task '{}' cwd must be relative to workspace root: {}",
                        task.name,
                        cwd.display()
                    ));
                }
                Some(cwd)
                    if cwd
                        .components()
                        .any(|component| component == Component::ParentDir) =>
                {
                    return Err(format!(
                        "Task '{}' cwd cannot escape workspace root: {}",
                        task.name,
                        cwd.display()
                    ));
                }
                Some(cwd) => {
                    let candidate = workspace_root.join(cwd);
                    if candidate.symlink_metadata().is_ok() {
                        let canonical_root = workspace_root.canonicalize().map_err(|error| {
                            format!(
                                "Task '{}' workspace root cannot be resolved: {} ({})",
                                task.name,
                                workspace_root.display(),
                                error
                            )
                        })?;
                        let canonical_candidate = candidate.canonicalize().map_err(|error| {
                            format!(
                                "Task '{}' cwd cannot be resolved: {} ({})",
                                task.name,
                                candidate.display(),
                                error
                            )
                        })?;
                        if !canonical_candidate.starts_with(&canonical_root) {
                            return Err(format!(
                                "Task '{}' cwd cannot escape workspace root through a symlink: {}",
                                task.name,
                                cwd.display()
                            ));
                        }
                    }
                    candidate
                }
            };
            resolved.context.cwd = Some(cwd);
            Ok(resolved)
        };

        let stages = self
            .stages
            .iter()
            .map(|stage| match stage {
                Stage::Serial(task) => resolve_task(task).map(Stage::Serial),
                Stage::Parallel { group, tasks } => tasks
                    .iter()
                    .map(resolve_task)
                    .collect::<Result<Vec<_>, _>>()
                    .map(|tasks| Stage::Parallel {
                        group: group.clone(),
                        tasks,
                    }),
            })
            .collect::<Result<Vec<_>, _>>()?;
        Ok(RunPlan { stages })
    }

    /// Builds the plan from parsed rules, preserving config order and group
    /// occurrence boundaries. Consecutive rules sharing the same non-empty
    /// `parallel` group form one occurrence; a serial rule, a different group
    /// name, or end of list closes the current occurrence (barrier).
    pub fn from_rules(rules: Vec<Rules>) -> RunPlan {
        let mut stages: Vec<Stage> = vec![];
        let mut open_group: Option<(String, Vec<TaskPlan>)> = None;
        let mut group_counts: std::collections::HashMap<String, usize> =
            std::collections::HashMap::new();

        let mut close_group = |group: String, tasks: Vec<TaskPlan>, stages: &mut Vec<Stage>| {
            let occurrence = group_counts.entry(group.clone()).or_insert(0);
            *occurrence += 1;
            let tasks = tasks
                .into_iter()
                .map(|mut task| {
                    task.group_occurrence = Some(format!("{}#{}", group, occurrence));
                    task
                })
                .collect();
            stages.push(Stage::Parallel { group, tasks });
        };

        for (position, rule) in rules.into_iter().enumerate() {
            let plan = TaskPlan {
                name: rule.name.clone(),
                position,
                commands: rule.command_lines(),
                parallel: rule.parallel().map(str::to_string),
                group_occurrence: None,
                context: TaskContext {
                    cwd: rule.cwd().map(PathBuf::from),
                    environment: rule.environment().clone(),
                },
                rule,
            };

            match (plan.parallel.as_deref(), open_group.as_mut()) {
                (Some(group), Some((open_name, tasks))) if group == open_name => {
                    tasks.push(plan);
                }
                (Some(group), _) => {
                    if let Some((open_name, tasks)) = open_group.take() {
                        close_group(open_name, tasks, &mut stages);
                    }
                    open_group = Some((group.to_string(), vec![plan]));
                }
                (None, _) => {
                    if let Some((group, tasks)) = open_group.take() {
                        close_group(group, tasks, &mut stages);
                    }
                    stages.push(Stage::Serial(plan));
                }
            }
        }

        if let Some((group, tasks)) = open_group.take() {
            close_group(group, tasks, &mut stages);
        }

        RunPlan { stages }
    }

    /// Filters tasks by a keep predicate without merging originally separate
    /// group occurrences: unmatched tasks are skipped, but a serial task
    /// removed between two occurrences keeps them separate (the barrier is
    /// implicit in the plan structure).
    pub fn filter<F>(self, keep: F) -> RunPlan
    where
        F: Fn(&Rules) -> bool,
    {
        let mut stages: Vec<Stage> = vec![];

        for stage in self.stages {
            match stage {
                Stage::Serial(plan) if keep(&plan.rule) => stages.push(Stage::Serial(plan)),
                Stage::Parallel { group, tasks } => {
                    let kept: Vec<TaskPlan> = tasks.into_iter().filter(|t| keep(&t.rule)).collect();
                    if !kept.is_empty() {
                        // Keep the occurrence even with one member: it may
                        // execute normally, but its barrier identity must not
                        // merge with another occurrence of the same name.
                        stages.push(Stage::Parallel { group, tasks: kept });
                    }
                }
                _ => {}
            }
        }

        RunPlan { stages }
    }

    /// Expands every task without flattening stages or group barriers.
    pub fn expand(&self, opts: &TemplateOptions) -> (RunPlan, Vec<String>) {
        let mut unknown = vec![];
        let stages = self
            .stages
            .iter()
            .map(|stage| match stage {
                Stage::Serial(task) => {
                    let (commands, task_unknown) = task.expand(opts);
                    unknown.extend(task_unknown);
                    let mut expanded = task.clone();
                    expanded.commands = commands;
                    Stage::Serial(expanded)
                }
                Stage::Parallel { group, tasks } => {
                    let tasks = tasks
                        .iter()
                        .map(|task| {
                            let (commands, task_unknown) = task.expand(opts);
                            unknown.extend(task_unknown);
                            let mut expanded = task.clone();
                            expanded.commands = commands;
                            expanded
                        })
                        .collect();
                    Stage::Parallel {
                        group: group.clone(),
                        tasks,
                    }
                }
            })
            .collect();
        (RunPlan { stages }, unknown)
    }

    /// Human diagnostics expose effective cwd and environment names, never
    /// environment values.
    pub fn context_summary(&self) -> String {
        self.stages
            .iter()
            .flat_map(|stage| match stage {
                Stage::Serial(task) => vec![task],
                Stage::Parallel { tasks, .. } => tasks.iter().collect(),
            })
            .map(|task| {
                let cwd = task
                    .context
                    .cwd
                    .as_ref()
                    .map(|cwd| cwd.display().to_string())
                    .unwrap_or_else(|| "<workspace>".to_owned());
                let keys = task
                    .context
                    .environment
                    .keys()
                    .cloned()
                    .collect::<Vec<_>>()
                    .join(", ");
                format!("task={} cwd={} env=[{}]", task.name, cwd, keys)
            })
            .collect::<Vec<_>>()
            .join("\n")
    }

    /// True when no stage remains.
    pub fn is_empty(&self) -> bool {
        self.stages.is_empty()
    }

    /// Flattens the plan into one ordered command list — the exact execution
    /// order a concurrency limit of one produces. Proves that a sequential
    /// run of the plan matches the legacy flat command list.
    pub fn commands(&self) -> Vec<CommandLine> {
        self.stages
            .iter()
            .flat_map(|stage| match stage {
                Stage::Serial(plan) => plan.commands.clone(),
                Stage::Parallel { tasks, .. } => {
                    tasks.iter().flat_map(|t| t.commands.clone()).collect()
                }
            })
            .collect()
    }

    /// All task identities in plan order.
    pub fn task_names(&self) -> Vec<String> {
        self.stages
            .iter()
            .flat_map(|stage| match stage {
                Stage::Serial(plan) => vec![plan.name.clone()],
                Stage::Parallel { tasks, .. } => tasks.iter().map(|t| t.name.clone()).collect(),
            })
            .collect()
    }

    /// Stable execution identity (contract §5): canonical SHA-256 over stage
    /// order and barriers, task/group identity, shell-vs-argv boundaries,
    /// resolved cwd, declared environment content, jobs, fail-fast, and
    /// schema version. Deterministic across restarts and map insertion
    /// order; environment **values** participate only as hash input and are
    /// never part of the displayable signature or persisted metadata.
    ///
    /// `DefaultHasher` is deliberately not used: its output is not a
    /// compatibility contract across processes or releases.
    pub fn execution_signature(&self, jobs: usize, fail_fast: bool) -> ExecutionSignature {
        execution_signature_for(self, jobs, fail_fast, SIGNATURE_SCHEMA_VERSION)
    }
}

/// Internal signature builder with an explicit schema version so tests can
/// prove a version bump invalidates old profiles.
fn execution_signature_for(
    plan: &RunPlan,
    jobs: usize,
    fail_fast: bool,
    schema_version: u64,
) -> ExecutionSignature {
    let mut canonical = CanonicalEncoder::new();
    canonical.u64(schema_version);
    canonical.u64(jobs as u64);
    canonical.bool(fail_fast);

    canonical.u64(plan.stages.len() as u64);
    for stage in &plan.stages {
        match stage {
            Stage::Serial(task) => {
                canonical.byte(0); // serial stage tag
                task.encode(&mut canonical);
            }
            Stage::Parallel { group, tasks } => {
                canonical.byte(1); // parallel stage tag
                canonical.string(group);
                canonical.u64(tasks.len() as u64);
                for task in tasks {
                    task.encode(&mut canonical);
                }
            }
        }
    }

    ExecutionSignature(hex(&Sha256::digest(&canonical.bytes)))
}

/// Canonical, ambiguity-free byte encoder for signature material. Lengths are
/// u64 little-endian prefixes so concatenations cannot collide; strings are
/// UTF-8 bytes; environment maps iterate in sorted key order (BTreeMap), so
/// insertion order cannot change the digest.
struct CanonicalEncoder {
    bytes: Vec<u8>,
}

impl CanonicalEncoder {
    fn new() -> Self {
        Self { bytes: Vec::new() }
    }

    fn u64(&mut self, value: u64) {
        self.bytes.extend_from_slice(&value.to_le_bytes());
    }

    fn byte(&mut self, value: u8) {
        self.bytes.push(value);
    }

    fn bool(&mut self, value: bool) {
        self.bytes.push(value as u8);
    }

    fn len_prefixed(&mut self, data: &[u8]) {
        self.u64(data.len() as u64);
        self.bytes.extend_from_slice(data);
    }

    fn string(&mut self, value: &str) {
        self.len_prefixed(value.as_bytes());
    }

    fn optional_string(&mut self, value: Option<&str>) {
        match value {
            Some(value) => {
                self.byte(1);
                self.string(value);
            }
            None => self.byte(0),
        }
    }
}

impl TaskPlan {
    /// Encodes one task's identity and execution-relevant content in canonical
    /// form: name, group/occurrence identity, command shell-vs-argv boundary
    /// and content, resolved cwd, and declared environment key/value content
    /// (values hashed, never displayed).
    fn encode(&self, canonical: &mut CanonicalEncoder) {
        canonical.string(&self.name);
        canonical.optional_string(self.parallel.as_deref());
        canonical.optional_string(self.group_occurrence.as_deref());
        canonical.u64(self.commands.len() as u64);
        for command in &self.commands {
            match command {
                CommandLine::Shell(command) => {
                    canonical.byte(0); // shell tag
                    canonical.string(command);
                }
                CommandLine::Argv(argv) => {
                    canonical.byte(1); // argv tag
                    canonical.u64(argv.len() as u64);
                    for argument in argv {
                        canonical.string(argument);
                    }
                }
            }
        }
        match &self.context.cwd {
            Some(cwd) => {
                canonical.byte(1);
                canonical.string(&cwd.display().to_string());
            }
            None => canonical.byte(0),
        }
        canonical.u64(self.context.environment.len() as u64);
        for (key, value) in &self.context.environment {
            canonical.string(key);
            canonical.string(value);
        }
    }
}

/// Lowercase hex encoding of a digest, stable and dependency-free.
pub(crate) fn hex(bytes: &[u8]) -> String {
    const DIGITS: &[u8; 16] = b"0123456789abcdef";
    let mut output = String::with_capacity(bytes.len() * 2);
    for byte in bytes {
        output.push(DIGITS[(byte >> 4) as usize] as char);
        output.push(DIGITS[(byte & 0x0f) as usize] as char);
    }
    output
}

impl RunOutcome {
    /// Derives the overall outcome from per-task outcomes in plan order.
    /// `group` is the named parallel group occurrence the task belongs to,
    /// None for serial tasks (TASK-0028).
    pub fn from_task_outcomes(outcomes: Vec<(String, Option<String>, TaskOutcome)>) -> RunOutcome {
        RunOutcome { tasks: outcomes }
    }

    /// True when every recorded task passed (no failures, no cancellation).
    pub fn is_success(&self) -> bool {
        self.tasks
            .iter()
            .all(|(_, _, outcome)| matches!(outcome, TaskOutcome::Passed))
    }

    /// True when at least one task failed.
    pub fn has_failures(&self) -> bool {
        self.tasks
            .iter()
            .any(|(_, _, outcome)| matches!(outcome, TaskOutcome::Failed { .. }))
    }

    /// True when the run was cancelled (restart replacement), even if some
    /// tasks had already passed.
    pub fn is_cancelled(&self) -> bool {
        self.tasks
            .iter()
            .any(|(_, _, outcome)| matches!(outcome, TaskOutcome::Cancelled))
    }

    /// All failure messages across tasks, in plan order.
    pub fn failures(&self) -> Vec<String> {
        self.tasks
            .iter()
            .flat_map(|(name, _, outcome)| match outcome {
                TaskOutcome::Failed { failures } => failures
                    .iter()
                    .map(|f| format!("{}: {}", name, f))
                    .collect::<Vec<String>>(),
                _ => vec![],
            })
            .collect()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn rule(name: &str, group: Option<&str>, run_on_init: bool) -> Rules {
        let rule = Rules::new(
            name.to_owned(),
            vec![format!("echo {}", name)],
            vec!["src/**".to_owned()],
            vec![],
            run_on_init,
        );
        match group {
            Some(group) => rule.with_parallel(group.to_owned()),
            None => rule,
        }
    }

    fn names(plan: &RunPlan) -> Vec<String> {
        plan.task_names()
    }

    #[test]
    fn ungrouped_tasks_stay_serial() {
        let plan = RunPlan::from_rules(vec![rule("A", None, false), rule("B", None, false)]);
        assert_eq!(names(&plan), vec!["A", "B"]);
        assert!(matches!(plan.stages[0], Stage::Serial(_)));
        assert!(matches!(plan.stages[1], Stage::Serial(_)));
    }

    #[test]
    fn consecutive_same_group_forms_one_occurrence() {
        let plan = RunPlan::from_rules(vec![
            rule("A", None, false),
            rule("B", Some("checks"), false),
            rule("C", Some("checks"), false),
            rule("D", None, false),
        ]);
        assert_eq!(names(&plan), vec!["A", "B", "C", "D"]);
        assert!(
            matches!(plan.stages[1], Stage::Parallel { ref group, ref tasks } if group == "checks" && tasks.len() == 2)
        );
    }

    #[test]
    fn reused_group_name_after_barrier_starts_new_occurrence() {
        let plan = RunPlan::from_rules(vec![
            rule("A", Some("x"), false),
            rule("B", None, false),
            rule("C", Some("x"), false),
        ]);
        let parallel_stages: Vec<&Stage> = plan
            .stages
            .iter()
            .filter(|s| matches!(s, Stage::Parallel { .. }))
            .collect();
        assert_eq!(parallel_stages.len(), 2, "reused name must not reconnect");
    }

    #[test]
    fn different_group_names_create_two_occurrences() {
        let plan = RunPlan::from_rules(vec![
            rule("A", Some("one"), false),
            rule("B", Some("two"), false),
        ]);
        let parallel_stages: Vec<&Stage> = plan
            .stages
            .iter()
            .filter(|s| matches!(s, Stage::Parallel { .. }))
            .collect();
        assert_eq!(parallel_stages.len(), 2);
    }

    #[test]
    fn task_plan_preserves_commands_and_identity() {
        let plan = RunPlan::from_rules(vec![rule("A", Some("g"), false)]);
        let Stage::Parallel { tasks, .. } = &plan.stages[0] else {
            panic!("expected parallel stage");
        };
        assert_eq!(tasks[0].name, "A");
        assert_eq!(tasks[0].position, 0);
        assert_eq!(tasks[0].parallel.as_deref(), Some("g"));
        assert_eq!(
            tasks[0].commands,
            vec![CommandLine::Shell("echo A".to_owned())]
        );
    }

    #[test]
    fn filtering_removes_unmatched_tasks_without_merging_occurrences() {
        let plan = RunPlan::from_rules(vec![
            rule("A", Some("x"), false),
            rule("B", None, false),
            rule("C", Some("x"), false),
        ]);
        // B is filtered out; the two x occurrences must stay separate.
        let filtered = plan.filter(|r| r.name != "B");
        let parallel_stages: Vec<&Stage> = filtered
            .stages
            .iter()
            .filter(|s| matches!(s, Stage::Parallel { .. }))
            .collect();
        assert_eq!(parallel_stages.len(), 2, "filter must not merge barriers");
        assert_eq!(names(&filtered), vec!["A", "C"]);
    }

    #[test]
    fn filtering_group_down_to_one_member_keeps_occurrence_but_runs_alone() {
        let plan = RunPlan::from_rules(vec![
            rule("A", Some("x"), false),
            rule("B", Some("x"), false),
        ]);
        let filtered = plan.filter(|r| r.name == "A");
        assert_eq!(names(&filtered), vec!["A"]);
        // The occurrence survives with one member; its barrier identity is
        // preserved so a later reuse of the name cannot reconnect.
        assert!(
            matches!(&filtered.stages[0], Stage::Parallel { group, tasks } if group == "x" && tasks.len() == 1)
        );
        // concurrency=1 flatten still yields exactly one command.
        assert_eq!(filtered.commands().len(), 1);
    }

    #[test]
    fn sequential_plan_commands_match_legacy_flat_order() {
        // concurrency=1 equivalence: plan.commands() equals legacy flatten.
        let rules = vec![
            rule("A", None, false),
            rule("B", Some("g"), false),
            rule("C", Some("g"), false),
            rule("D", None, false),
        ];
        let plan = RunPlan::from_rules(rules.clone());

        let legacy: Vec<CommandLine> = rules.iter().flat_map(|r| r.command_lines()).collect();
        assert_eq!(plan.commands(), legacy);
        assert_eq!(plan.commands().len(), 4);
    }

    #[test]
    fn serial_tasks_have_no_group_occurrence_identity() {
        let plan = RunPlan::from_rules(vec![rule("A", None, false), rule("B", None, false)]);
        for stage in &plan.stages {
            let Stage::Serial(task) = stage else {
                panic!("expected serial stage");
            };
            assert_eq!(task.group_occurrence, None);
        }
    }

    #[test]
    fn one_parallel_occurrence_is_numbered_one() {
        let plan = RunPlan::from_rules(vec![
            rule("B", Some("checks"), false),
            rule("C", Some("checks"), false),
        ]);
        let Stage::Parallel { tasks, .. } = &plan.stages[0] else {
            panic!("expected parallel stage");
        };
        for task in tasks {
            assert_eq!(task.group_occurrence.as_deref(), Some("checks#1"));
        }
    }

    #[test]
    fn reused_group_name_after_barrier_gets_a_new_occurrence_id() {
        let plan = RunPlan::from_rules(vec![
            rule("A", Some("x"), false),
            rule("B", None, false),
            rule("C", Some("x"), false),
        ]);
        let parallel_stages: Vec<&Stage> = plan
            .stages
            .iter()
            .filter(|s| matches!(s, Stage::Parallel { .. }))
            .collect();
        assert_eq!(parallel_stages.len(), 2);
        let Stage::Parallel { tasks: first, .. } = &plan.stages[0] else {
            panic!("parallel stage");
        };
        let Stage::Parallel { tasks: second, .. } = &plan.stages[2] else {
            panic!("parallel stage");
        };
        assert_eq!(first[0].group_occurrence.as_deref(), Some("x#1"));
        assert_eq!(second[0].group_occurrence.as_deref(), Some("x#2"));
    }

    #[test]
    fn filtering_keeps_group_occurrence_identity_stable() {
        let plan = RunPlan::from_rules(vec![
            rule("A", Some("x"), false),
            rule("B", Some("x"), false),
        ]);
        let filtered = plan.filter(|r| r.name == "B");
        let Stage::Parallel { tasks, .. } = &filtered.stages[0] else {
            panic!("expected parallel stage");
        };
        assert_eq!(tasks[0].group_occurrence.as_deref(), Some("x#1"));
    }

    #[test]
    fn outcome_combination_is_order_independent() {
        let a = RunOutcome::from_task_outcomes(vec![
            ("t1".to_owned(), None, TaskOutcome::Passed),
            (
                "t2".to_owned(),
                None,
                TaskOutcome::Failed {
                    failures: vec!["boom".to_owned()],
                },
            ),
        ]);
        let b = RunOutcome::from_task_outcomes(vec![
            (
                "t2".to_owned(),
                None,
                TaskOutcome::Failed {
                    failures: vec!["boom".to_owned()],
                },
            ),
            ("t1".to_owned(), None, TaskOutcome::Passed),
        ]);
        assert!(!a.is_success());
        assert!(a.has_failures());
        assert!(!a.is_cancelled());
        assert_eq!(a.failures(), b.failures());
    }

    #[test]
    fn outcome_combination_keeps_group_identity_keyed_by_task() {
        // TASK-0028: the summary identifies group and every task; combining
        // outcomes is keyed by task identity, and parallel-group ordering is
        // explicitly unspecified (reversed inputs reduce identically).
        let a = RunOutcome::from_task_outcomes(vec![
            (
                "lint".to_owned(),
                Some("checks".to_owned()),
                TaskOutcome::Passed,
            ),
            (
                "test".to_owned(),
                Some("checks".to_owned()),
                TaskOutcome::Failed {
                    failures: vec!["boom".to_owned()],
                },
            ),
        ]);
        let b = RunOutcome::from_task_outcomes(vec![
            (
                "test".to_owned(),
                Some("checks".to_owned()),
                TaskOutcome::Failed {
                    failures: vec!["boom".to_owned()],
                },
            ),
            (
                "lint".to_owned(),
                Some("checks".to_owned()),
                TaskOutcome::Passed,
            ),
        ]);
        assert_eq!(a.tasks.len(), 2);
        // Every task keeps its group identity regardless of completion order.
        assert!(a
            .tasks
            .iter()
            .all(|(_, group, _)| group.as_deref() == Some("checks")));
        assert_eq!(a.failures(), b.failures());
        assert_eq!(
            a.tasks
                .iter()
                .find(|(name, _, _)| name == "test")
                .map(|(_, group, _)| group.clone()),
            Some(Some("checks".to_owned()))
        );
    }

    #[test]
    fn cancelled_run_is_never_success() {
        let outcome = RunOutcome::from_task_outcomes(vec![
            ("t1".to_owned(), None, TaskOutcome::Passed),
            ("t2".to_owned(), None, TaskOutcome::Cancelled),
        ]);
        assert!(!outcome.is_success());
        assert!(outcome.is_cancelled());
        assert!(!outcome.has_failures());
    }

    #[test]
    fn argv_rules_keep_argv_commands_in_plan() {
        let argv_rule = Rules::from_argv(
            "fmt".to_owned(),
            vec![
                "cargo".to_owned(),
                "fmt".to_owned(),
                "{{filepath}}".to_owned(),
            ],
            vec!["src/**".to_owned()],
            vec![],
            false,
        );
        let plan = RunPlan::from_rules(vec![argv_rule]);
        let Stage::Serial(task) = &plan.stages[0] else {
            panic!("expected serial stage");
        };
        assert_eq!(
            task.commands,
            vec![CommandLine::Argv(vec![
                "cargo".to_owned(),
                "fmt".to_owned(),
                "{{filepath}}".to_owned()
            ])]
        );
        assert_eq!(plan.commands().len(), 1);
    }

    #[test]
    fn resolves_task_context_and_expands_relative_path_from_task_cwd() {
        let mut environment = BTreeMap::new();
        environment.insert("ROLE".to_owned(), "web".to_owned());
        let rule = Rules::new(
            "web".to_owned(),
            vec!["echo {{relative_filepath}}".to_owned()],
            vec!["packages/**".to_owned()],
            vec![],
            false,
        )
        .with_execution_context(Some("packages/web app".to_owned()), environment.clone());
        let root = PathBuf::from("/tmp/work space");
        let plan = RunPlan::from_rules(vec![rule])
            .resolve_context(&root)
            .expect("relative cwd");
        let (expanded, unknown) = plan.expand(&TemplateOptions {
            filepath: Some("/tmp/work space/packages/web app/src/main.rs".to_owned()),
            current_dir: root.display().to_string(),
        });
        let Stage::Serial(task) = &expanded.stages[0] else {
            panic!("serial task");
        };

        assert_eq!(task.context.cwd, Some(root.join("packages/web app")));
        assert_eq!(task.context.environment, environment);
        assert_eq!(
            task.commands,
            vec![CommandLine::Shell("echo src/main.rs".to_owned())]
        );
        assert!(unknown.is_empty());
    }

    #[test]
    fn rejects_absolute_and_parent_task_working_directories() {
        for cwd in ["/tmp/outside", "../outside"] {
            let rule = Rules::new(
                "unsafe".to_owned(),
                vec!["true".to_owned()],
                vec!["src/**".to_owned()],
                vec![],
                false,
            )
            .with_execution_context(Some(cwd.to_owned()), BTreeMap::new());
            let error = RunPlan::from_rules(vec![rule])
                .resolve_context(Path::new("/workspace"))
                .expect_err("cwd escape must fail");
            assert!(error.contains("Task 'unsafe' cwd"), "unexpected: {error}");
        }
    }

    #[cfg(unix)]
    #[test]
    fn rejects_task_working_directory_symlinks_outside_workspace() {
        use std::os::unix::fs::symlink;

        let fixture =
            std::env::temp_dir().join(format!("funzzy-task-cwd-escape-{}", std::process::id()));
        let workspace = fixture.join("workspace");
        let outside = fixture.join("outside");
        let _ = std::fs::remove_dir_all(&fixture);
        std::fs::create_dir_all(&workspace).unwrap();
        std::fs::create_dir_all(&outside).unwrap();
        symlink(&outside, workspace.join("linked-outside")).unwrap();

        let rule = Rules::new(
            "unsafe".to_owned(),
            vec!["true".to_owned()],
            vec!["src/**".to_owned()],
            vec![],
            false,
        )
        .with_execution_context(Some("linked-outside".to_owned()), BTreeMap::new());
        let error = RunPlan::from_rules(vec![rule])
            .resolve_context(&workspace)
            .expect_err("cwd symlink escape must fail");

        assert!(error.contains("Task 'unsafe' cwd"), "unexpected: {error}");
        std::fs::remove_dir_all(fixture).unwrap();
    }

    #[test]
    fn context_summary_redacts_environment_values() {
        let rule = Rules::new(
            "secret".to_owned(),
            vec!["true".to_owned()],
            vec!["src/**".to_owned()],
            vec![],
            false,
        )
        .with_execution_context(
            Some("service".to_owned()),
            BTreeMap::from([("TOKEN".to_owned(), "do-not-print".to_owned())]),
        );
        let summary = RunPlan::from_rules(vec![rule]).context_summary();
        assert!(summary.contains("env=[TOKEN]"));
        assert!(!summary.contains("do-not-print"));
    }

    #[test]
    fn empty_plan_reports_empty() {
        assert!(RunPlan::default().is_empty());
        assert!(RunPlan::from_rules(vec![]).is_empty());
    }

    #[test]
    fn task_plan_expand_preserves_path_values_and_unknown_variables() {
        let rule = Rules::new(
            "t".to_owned(),
            vec!["echo {{filepath}} {{relative_filepath}} {{oops}}".to_owned()],
            vec!["src/**".to_owned()],
            vec![],
            false,
        );
        let plan = RunPlan::from_rules(vec![rule]);
        let Stage::Serial(task) = &plan.stages[0] else {
            panic!("expected serial stage");
        };

        let opts = TemplateOptions {
            filepath: Some("/root/src/main.rs".to_owned()),
            current_dir: "/root".to_owned(),
        };
        let (expanded, unknown) = task.expand(&opts);

        // Expanded path values are preserved exactly.
        assert_eq!(
            expanded,
            vec![CommandLine::Shell(
                "echo /root/src/main.rs src/main.rs {{oops}}".to_owned()
            )]
        );
        // Unknown template variables are reported without side effects.
        assert_eq!(unknown, vec!["oops".to_owned()]);
    }

    // --- Execution signature (contract §5, matrix in §8) ---

    fn env_rule(name: &str, cwd: Option<&str>, env: BTreeMap<String, String>) -> Rules {
        Rules::new(
            name.to_owned(),
            vec!["echo hi".to_owned()],
            vec!["src/**".to_owned()],
            vec![],
            false,
        )
        .with_execution_context(cwd.map(str::to_owned), env)
    }

    #[test]
    fn signature_is_deterministic_for_identical_plan() {
        let plan = RunPlan::from_rules(vec![rule("A", None, false), rule("B", Some("g"), false)]);
        let first = plan.execution_signature(4, false);
        let second = plan.execution_signature(4, false);
        assert_eq!(first, second);
        assert_eq!(first.to_string().len(), 64, "sha256 lowercase hex");
    }

    #[test]
    fn signature_ignores_environment_map_insertion_order() {
        let mut env_a = BTreeMap::new();
        env_a.insert("A".to_owned(), "1".to_owned());
        env_a.insert("B".to_owned(), "2".to_owned());
        let mut env_b = BTreeMap::new();
        env_b.insert("B".to_owned(), "2".to_owned());
        env_b.insert("A".to_owned(), "1".to_owned());
        let plan_a = RunPlan::from_rules(vec![env_rule("A", None, env_a)]);
        let plan_b = RunPlan::from_rules(vec![env_rule("A", None, env_b)]);
        assert_eq!(
            plan_a.execution_signature(1, false),
            plan_b.execution_signature(1, false)
        );
    }

    #[test]
    fn signature_changes_with_command_content() {
        let a = Rules::new(
            "t".to_owned(),
            vec!["make build".to_owned()],
            vec!["src/**".to_owned()],
            vec![],
            false,
        );
        let b = Rules::new(
            "t".to_owned(),
            vec!["make test".to_owned()],
            vec!["src/**".to_owned()],
            vec![],
            false,
        );
        assert_ne!(
            RunPlan::from_rules(vec![a]).execution_signature(1, false),
            RunPlan::from_rules(vec![b]).execution_signature(1, false)
        );
    }

    #[test]
    fn signature_distinguishes_shell_from_argv_boundaries() {
        let shell = Rules::new(
            "t".to_owned(),
            vec!["cargo fmt".to_owned()],
            vec!["src/**".to_owned()],
            vec![],
            false,
        );
        let argv = Rules::from_argv(
            "t".to_owned(),
            vec!["cargo".to_owned(), "fmt".to_owned()],
            vec!["src/**".to_owned()],
            vec![],
            false,
        );
        assert_ne!(
            RunPlan::from_rules(vec![shell]).execution_signature(1, false),
            RunPlan::from_rules(vec![argv]).execution_signature(1, false)
        );
    }

    #[test]
    fn signature_changes_with_resolved_cwd() {
        let plan_a = RunPlan::from_rules(vec![env_rule("A", Some("packages/a"), BTreeMap::new())]);
        let plan_b = RunPlan::from_rules(vec![env_rule("A", Some("packages/b"), BTreeMap::new())]);
        assert_ne!(
            plan_a.execution_signature(1, false),
            plan_b.execution_signature(1, false)
        );
    }

    #[test]
    fn signature_changes_with_environment_content() {
        let plan_a = RunPlan::from_rules(vec![env_rule(
            "A",
            None,
            BTreeMap::from([("ROLE".to_owned(), "web".to_owned())]),
        )]);
        let plan_b = RunPlan::from_rules(vec![env_rule(
            "A",
            None,
            BTreeMap::from([("ROLE".to_owned(), "worker".to_owned())]),
        )]);
        assert_ne!(
            plan_a.execution_signature(1, false),
            plan_b.execution_signature(1, false)
        );
    }

    #[test]
    fn signature_changes_with_topology_and_group_identity() {
        // Two serial tasks vs one parallel group of two: topology differs.
        let serial = RunPlan::from_rules(vec![rule("A", None, false), rule("B", None, false)]);
        let parallel = RunPlan::from_rules(vec![
            rule("A", Some("g"), false),
            rule("B", Some("g"), false),
        ]);
        assert_ne!(
            serial.execution_signature(2, false),
            parallel.execution_signature(2, false)
        );

        // Group name change differs even with same task names.
        let group_one = RunPlan::from_rules(vec![rule("A", Some("one"), false)]);
        let group_two = RunPlan::from_rules(vec![rule("A", Some("two"), false)]);
        assert_ne!(
            group_one.execution_signature(1, false),
            group_two.execution_signature(1, false)
        );

        // Reused group name after a barrier gets a new occurrence identity.
        let split = RunPlan::from_rules(vec![
            rule("A", Some("x"), false),
            rule("B", None, false),
            rule("C", Some("x"), false),
        ]);
        let contiguous = RunPlan::from_rules(vec![
            rule("A", Some("x"), false),
            rule("B", Some("x"), false),
            rule("C", Some("x"), false),
        ]);
        assert_ne!(
            split.execution_signature(1, false),
            contiguous.execution_signature(1, false)
        );
    }

    #[test]
    fn signature_changes_with_jobs_and_fail_fast() {
        let plan = RunPlan::from_rules(vec![rule("A", None, false), rule("B", None, false)]);
        assert_ne!(
            plan.execution_signature(1, false),
            plan.execution_signature(2, false)
        );
        assert_ne!(
            plan.execution_signature(1, false),
            plan.execution_signature(1, true)
        );
    }

    #[test]
    fn signature_changes_with_schema_version() {
        let plan = RunPlan::from_rules(vec![rule("A", None, false)]);
        assert_ne!(
            execution_signature_for(&plan, 1, false, 1),
            execution_signature_for(&plan, 1, false, 2)
        );
    }

    #[test]
    fn signature_hashes_secret_environment_values_without_displaying_them() {
        let secret = "super-secret-token-123";
        let plan = RunPlan::from_rules(vec![env_rule(
            "A",
            None,
            BTreeMap::from([("API_TOKEN".to_owned(), secret.to_owned())]),
        )]);
        let signature = plan.execution_signature(1, false);
        // The signature is opaque hex: the secret never appears in it, and
        // the signature carries no readable command/env content.
        assert!(!signature.to_string().contains(secret));
        assert!(!signature.to_string().contains("API_TOKEN"));
        assert!(!signature.to_string().contains("echo"));
    }
}
