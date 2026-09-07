//! Filesystem adapter for resolving task working directories.
//!
//! Lexical cwd policy lives in `plan`; this module owns only filesystem
//! observation (existence and symlink containment) around that pure result.

use crate::plan::{RunPlan, Stage, TaskPlan};
use std::path::Path;

/// Resolves task cwd candidates and applies filesystem-dependent containment
/// checks while preserving the plan's existing error behavior.
pub(crate) fn resolve_context(plan: &RunPlan, workspace_root: &Path) -> Result<RunPlan, String> {
    let resolved = plan.resolve_context(workspace_root)?;
    for (source, candidate) in task_pairs(plan, &resolved) {
        if let Some(requested) = source.context.cwd.as_deref() {
            observe_task_cwd(
                &source.name,
                workspace_root,
                requested,
                candidate.context.cwd.as_deref().expect("resolved cwd"),
            )?;
        }
    }
    Ok(resolved)
}

fn task_pairs<'a>(source: &'a RunPlan, resolved: &'a RunPlan) -> Vec<(&'a TaskPlan, &'a TaskPlan)> {
    source
        .stages
        .iter()
        .zip(&resolved.stages)
        .flat_map(|(source, resolved)| match (source, resolved) {
            (Stage::Serial(source), Stage::Serial(resolved)) => vec![(source, resolved)],
            (
                Stage::Parallel { tasks: source, .. },
                Stage::Parallel {
                    tasks: resolved, ..
                },
            ) => source.iter().zip(resolved).collect(),
            _ => unreachable!("pure context resolution preserves plan topology"),
        })
        .collect()
}

/// Performs the filesystem-dependent part of cwd validation. Missing paths
/// remain valid candidates; the executor performs the final existence check
/// immediately before spawning a task, preserving the existing behavior.
fn observe_task_cwd(
    task_name: &str,
    workspace_root: &Path,
    requested: &Path,
    candidate: &Path,
) -> Result<(), String> {
    if candidate.symlink_metadata().is_ok() {
        let canonical_root = workspace_root.canonicalize().map_err(|error| {
            format!(
                "Task '{}' workspace root cannot be resolved: {} ({})",
                task_name,
                workspace_root.display(),
                error
            )
        })?;
        let canonical_candidate = candidate.canonicalize().map_err(|error| {
            format!(
                "Task '{}' cwd cannot be resolved: {} ({})",
                task_name,
                candidate.display(),
                error
            )
        })?;
        if !canonical_candidate.starts_with(&canonical_root) {
            return Err(format!(
                "Task '{}' cwd cannot escape workspace root through a symlink: {}",
                task_name,
                requested.display()
            ));
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::rules::Rules;
    use std::collections::BTreeMap;

    fn plan_with_cwd(cwd: &str) -> RunPlan {
        RunPlan::from_rules(vec![Rules::new(
            "task".to_owned(),
            vec!["true".to_owned()],
            vec!["src/**".to_owned()],
            vec![],
            false,
        )
        .with_execution_context(Some(cwd.to_owned()), BTreeMap::new())])
    }

    #[test]
    fn missing_task_directory_stays_a_candidate_until_spawn() {
        let fixture = std::env::temp_dir().join(format!(
            "funzzy-task-cwd-adapter-missing-{}",
            std::process::id()
        ));
        let _ = std::fs::remove_dir_all(&fixture);

        let resolved = resolve_context(&plan_with_cwd("does-not-exist"), &fixture)
            .expect("missing paths remain valid candidates");
        let Stage::Serial(task) = &resolved.stages[0] else {
            panic!("expected serial task");
        };
        assert_eq!(task.context.cwd, Some(fixture.join("does-not-exist")));
    }

    #[test]
    fn existing_task_directory_inside_workspace_is_accepted() {
        let fixture = std::env::temp_dir().join(format!(
            "funzzy-task-cwd-adapter-inside-{}",
            std::process::id()
        ));
        let workspace = fixture.join("workspace");
        std::fs::create_dir_all(workspace.join("packages/web")).unwrap();

        let resolved = resolve_context(&plan_with_cwd("packages/web"), &workspace)
            .expect("cwd inside workspace must be accepted");
        let Stage::Serial(task) = &resolved.stages[0] else {
            panic!("expected serial task");
        };
        assert_eq!(task.context.cwd, Some(workspace.join("packages/web")));
        std::fs::remove_dir_all(fixture).unwrap();
    }

    #[cfg(unix)]
    #[test]
    fn symlinked_task_directory_outside_workspace_is_rejected() {
        use std::os::unix::fs::symlink;

        let fixture = std::env::temp_dir().join(format!(
            "funzzy-task-cwd-adapter-escape-{}",
            std::process::id()
        ));
        let workspace = fixture.join("workspace");
        let outside = fixture.join("outside");
        let _ = std::fs::remove_dir_all(&fixture);
        std::fs::create_dir_all(&workspace).unwrap();
        std::fs::create_dir_all(&outside).unwrap();
        symlink(&outside, workspace.join("linked-outside")).unwrap();

        let error = resolve_context(&plan_with_cwd("linked-outside"), &workspace)
            .expect_err("symlink escape must fail");

        assert!(error.contains("Task 'task' cwd"), "unexpected: {error}");
        std::fs::remove_dir_all(fixture).unwrap();
    }
}
