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
    /// The original rule, kept for matching/presentation consumers.
    pub rule: Rules,
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
    /// Task identity -> outcome, preserving plan order.
    pub tasks: Vec<(String, TaskOutcome)>,
}

impl TaskPlan {
    /// Expands template variables in this task's commands for the given
    /// path options, returning the expanded commands and any unknown
    /// variables. Pure: no process, stdout, or control-socket side effects.
    pub fn expand(&self, opts: &TemplateOptions) -> (Vec<CommandLine>, Vec<String>) {
        let mut unknown = vec![];
        let expanded = self
            .commands
            .iter()
            .map(|cmd| {
                let out = template::template_line(cmd.clone(), opts.clone());
                unknown.extend(out.unknown_variables);
                out.command
            })
            .collect();
        (expanded, unknown)
    }
}

impl RunPlan {
    /// Builds the plan from parsed rules, preserving config order and group
    /// occurrence boundaries. Consecutive rules sharing the same non-empty
    /// `parallel` group form one occurrence; a serial rule, a different group
    /// name, or end of list closes the current occurrence (barrier).
    pub fn from_rules(rules: Vec<Rules>) -> RunPlan {
        let mut stages: Vec<Stage> = vec![];
        let mut open_group: Option<(String, Vec<TaskPlan>)> = None;

        for (position, rule) in rules.into_iter().enumerate() {
            let plan = TaskPlan {
                name: rule.name.clone(),
                position,
                commands: rule.command_lines(),
                parallel: rule.parallel().map(str::to_string),
                rule,
            };

            match (plan.parallel.as_deref(), open_group.as_mut()) {
                (Some(group), Some((open_name, tasks))) if group == open_name => {
                    tasks.push(plan);
                }
                (Some(group), _) => {
                    if let Some((open_name, tasks)) = open_group.take() {
                        stages.push(Stage::Parallel {
                            group: open_name,
                            tasks,
                        });
                    }
                    open_group = Some((group.to_string(), vec![plan]));
                }
                (None, _) => {
                    if let Some((group, tasks)) = open_group.take() {
                        stages.push(Stage::Parallel { group, tasks });
                    }
                    stages.push(Stage::Serial(plan));
                }
            }
        }

        if let Some((group, tasks)) = open_group.take() {
            stages.push(Stage::Parallel { group, tasks });
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
}

impl RunOutcome {
    /// Derives the overall outcome from per-task outcomes in plan order.
    pub fn from_task_outcomes(outcomes: Vec<(String, TaskOutcome)>) -> RunOutcome {
        RunOutcome { tasks: outcomes }
    }

    /// True when every recorded task passed (no failures, no cancellation).
    pub fn is_success(&self) -> bool {
        self.tasks
            .iter()
            .all(|(_, outcome)| matches!(outcome, TaskOutcome::Passed))
    }

    /// True when at least one task failed.
    pub fn has_failures(&self) -> bool {
        self.tasks
            .iter()
            .any(|(_, outcome)| matches!(outcome, TaskOutcome::Failed { .. }))
    }

    /// True when the run was cancelled (restart replacement), even if some
    /// tasks had already passed.
    pub fn is_cancelled(&self) -> bool {
        self.tasks
            .iter()
            .any(|(_, outcome)| matches!(outcome, TaskOutcome::Cancelled))
    }

    /// All failure messages across tasks, in plan order.
    pub fn failures(&self) -> Vec<String> {
        self.tasks
            .iter()
            .flat_map(|(name, outcome)| match outcome {
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
        // jobs=1 flatten still yields exactly one command.
        assert_eq!(filtered.commands().len(), 1);
    }

    #[test]
    fn sequential_plan_commands_match_legacy_flat_order() {
        // jobs=1 equivalence: plan.commands() equals the legacy flatten.
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
    fn outcome_combination_is_order_independent() {
        let a = RunOutcome::from_task_outcomes(vec![
            ("t1".to_owned(), TaskOutcome::Passed),
            (
                "t2".to_owned(),
                TaskOutcome::Failed {
                    failures: vec!["boom".to_owned()],
                },
            ),
        ]);
        let b = RunOutcome::from_task_outcomes(vec![
            (
                "t2".to_owned(),
                TaskOutcome::Failed {
                    failures: vec!["boom".to_owned()],
                },
            ),
            ("t1".to_owned(), TaskOutcome::Passed),
        ]);
        assert!(!a.is_success());
        assert!(a.has_failures());
        assert!(!a.is_cancelled());
        assert_eq!(a.failures(), b.failures());
    }

    #[test]
    fn cancelled_run_is_never_success() {
        let outcome = RunOutcome::from_task_outcomes(vec![
            ("t1".to_owned(), TaskOutcome::Passed),
            ("t2".to_owned(), TaskOutcome::Cancelled),
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
}
