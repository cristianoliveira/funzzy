use funzzy::executor::{Executor, RunMetadata, SystemClock, SystemProcessRunner};
use funzzy::plan::RunPlan;
use funzzy::rules::Rules;
use std::collections::BTreeMap;
use std::sync::Arc;

fn rule(
    name: &str,
    group: Option<&str>,
    cwd: Option<&str>,
    environment: BTreeMap<String, String>,
    command: &str,
) -> Rules {
    let rule = Rules::new(
        name.to_owned(),
        vec![command.to_owned()],
        vec!["src/**".to_owned()],
        vec![],
        false,
    )
    .with_execution_context(cwd.map(str::to_owned), environment);
    match group {
        Some(group) => rule.with_parallel(group.to_owned()),
        None => rule,
    }
}

fn executor(jobs: usize) -> Executor {
    Executor::new(
        Arc::new(SystemProcessRunner),
        Arc::new(SystemClock),
        jobs,
        Arc::new(|_| {}),
        false,
        false,
    )
    .unwrap()
}

#[test]
fn parallel_tasks_isolate_cwd_and_environment_without_global_mutation() {
    let root = std::env::temp_dir().join(format!(
        "funzzy task context with spaces {}",
        std::process::id()
    ));
    let _ = std::fs::remove_dir_all(&root);
    std::fs::create_dir_all(root.join("service a")).unwrap();
    std::fs::create_dir_all(root.join("service b")).unwrap();

    let variable = "FUNZZY_TASK_0032_CONTEXT";
    let empty_variable = "FUNZZY_TASK_0032_EMPTY";
    assert!(
        std::env::var(variable).is_err() && std::env::var(empty_variable).is_err(),
        "test variables must start unset"
    );
    let rules = vec![
        rule(
            "a",
            Some("services"),
            Some("service a"),
            BTreeMap::from([
                (variable.to_owned(), "alpha".to_owned()),
                (empty_variable.to_owned(), "".to_owned()),
            ]),
            "test \"$FUNZZY_TASK_0032_CONTEXT\" = alpha && test -n \"$PATH\" && test \"${FUNZZY_TASK_0032_EMPTY+x}\" = x && test -z \"$FUNZZY_TASK_0032_EMPTY\" && printf alpha > result.txt",
        ),
        rule(
            "b",
            Some("services"),
            Some("service b"),
            BTreeMap::from([(variable.to_owned(), "beta".to_owned())]),
            "test \"$FUNZZY_TASK_0032_CONTEXT\" = beta && printf beta > result.txt",
        ),
        rule(
            "after",
            None,
            None,
            BTreeMap::new(),
            "test \"${FUNZZY_TASK_0032_CONTEXT-unset}\" = unset && test \"${FUNZZY_TASK_0032_EMPTY-unset}\" = unset && printf clean > later.txt",
        ),
    ];
    let plan = RunPlan::from_rules(rules).resolve_context(&root).unwrap();

    let completed = executor(2).run_to_completion(RunMetadata::new(1, "test"), plan);

    assert!(completed.outcome.is_success(), "{:?}", completed.outcome);
    assert_eq!(
        std::fs::read_to_string(root.join("service a/result.txt")).unwrap(),
        "alpha"
    );
    assert_eq!(
        std::fs::read_to_string(root.join("service b/result.txt")).unwrap(),
        "beta"
    );
    assert_eq!(
        std::fs::read_to_string(root.join("later.txt")).unwrap(),
        "clean"
    );
    assert!(
        std::env::var(variable).is_err() && std::env::var(empty_variable).is_err(),
        "task env must not leak globally"
    );
    std::fs::remove_dir_all(root).unwrap();
}

#[test]
fn missing_task_directory_fails_before_command_spawn() {
    let root = std::env::temp_dir().join(format!(
        "funzzy-task-context-missing-{}",
        std::process::id()
    ));
    let _ = std::fs::remove_dir_all(&root);
    std::fs::create_dir_all(&root).unwrap();
    let plan = RunPlan::from_rules(vec![rule(
        "missing",
        None,
        Some("does-not-exist"),
        BTreeMap::new(),
        "printf spawned > should-not-exist.txt",
    )])
    .resolve_context(&root)
    .unwrap();

    let completed = executor(1).run_to_completion(RunMetadata::new(1, "test"), plan);

    assert!(completed.outcome.has_failures());
    assert!(completed
        .outcome
        .failures()
        .iter()
        .any(|failure| failure.contains("Task 'missing' cwd is missing or not a directory")));
    assert!(!root.join("should-not-exist.txt").exists());
    std::fs::remove_dir_all(root).unwrap();
}
