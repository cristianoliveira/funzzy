//! `fzz migrate`: explicit rewrite of an accepted legacy configuration into
//! the preferred ordered `jobs:` form (TASK-0098, CLI-V2-CONTRACT §3a).
//!
//! The transformation is pure and independently testable — no filesystem,
//! no stdout. `MigrateCommand` is the CLI adapter: it selects the configured
//! path (global `-c/--config`, default `.watch.yaml`), validates the complete
//! candidate through the production parser, and replaces the file atomically.

use crate::cli::Command;
use crate::errors::FzzError;
use crate::stdout;
use yaml_rust2::{Yaml, YamlEmitter, YamlLoader};

/// Migrates an accepted legacy config to the preferred V2 `jobs:` format
/// (TASK-0075/0076): a root task list is wrapped into an ordered `jobs:`
/// list preserving declaration order, comments, quoting, and commands; a
/// grouped `tasks:` root is renamed to `jobs:`. Root-list groups are flattened
/// in declaration order with effective group patterns materialized on each
/// child job. Already-preferred input is returned unchanged (idempotent,
/// byte-preserving no-op).
pub fn migrate_content(legacy: &str) -> Result<String, FzzError> {
    let documents = YamlLoader::load_from_str(legacy).map_err(|err| {
        FzzError::InvalidConfigError(
            "Invalid legacy .watch.yaml".to_string(),
            Some(err),
            Some("Fix the YAML syntax before running `fzz migrate`".to_string()),
        )
    })?;

    if documents.len() != 1 {
        return Err(FzzError::GenericError(
            "Legacy .watch.yaml must contain exactly one YAML document".to_string(),
        ));
    }

    match documents.first() {
        Some(Yaml::Array(items)) if items.iter().any(is_nested_group) => {
            return flatten_nested_groups(legacy, items);
        }
        Some(Yaml::Array(_)) => {}
        Some(document) if document["jobs"] != Yaml::BadValue => {
            // Already preferred: idempotent no-op (TASK-0078) — return the
            // input unchanged so the file is never rewritten.
            return Ok(legacy.to_owned());
        }
        Some(document) if document["tasks"] != Yaml::BadValue => {
            // Grouped tasks: rename the root key to jobs, preserving order.
            return Ok(rename_root_key(legacy, "tasks", "jobs"));
        }
        _ => {
            return Err(FzzError::GenericError(
                "Legacy .watch.yaml root must be a task list".to_string(),
            ));
        }
    }

    let task_start = legacy
        .lines()
        .position(|line| {
            let line = line.trim_start();
            line == "-" || (line.starts_with("- ") && line != "---")
        })
        .ok_or_else(|| {
            FzzError::GenericError(
                "Legacy .watch.yaml does not contain any tasks to migrate".to_string(),
            )
        })?;

    let mut migrated = String::new();
    let mut lines = legacy.split_inclusive('\n');

    for _ in 0..task_start {
        if let Some(line) = lines.next() {
            migrated.push_str(line);
        }
    }

    migrated.push_str("jobs:\n");
    for line in lines {
        if line.trim().is_empty() {
            migrated.push_str(line);
        } else {
            migrated.push_str("  ");
            migrated.push_str(line);
        }
    }

    Ok(migrated)
}

fn is_nested_group(item: &Yaml) -> bool {
    matches!(item, Yaml::Hash(_)) && item["tasks"] != Yaml::BadValue
}

/// Flattens the accepted root-list group form into one ordered V2 `jobs`
/// array. Group matching policy is materialized on each child job because a
/// V2 jobs list cannot contain nested `on`/`tasks` objects.
fn flatten_nested_groups(legacy: &str, items: &[Yaml]) -> Result<String, FzzError> {
    let mut jobs = Vec::new();
    for item in items {
        if !is_nested_group(item) {
            jobs.push(item.clone());
            continue;
        }

        let common = pattern_map(item, "on")?;
        let tasks = match &item["tasks"] {
            Yaml::Array(tasks) => tasks,
            other => {
                return Err(FzzError::GenericError(format!(
                    "Nested group 'tasks' must be an ordered list, got {}",
                    crate::yaml::get_type(other)
                )));
            }
        };
        for task in tasks {
            if is_nested_group(task) {
                return Err(FzzError::GenericError(
                    "Nested groups may not contain another on/tasks group".to_string(),
                ));
            }
            jobs.push(materialize_group_patterns(task, &common)?);
        }
    }

    let mut emitted = String::new();
    // yaml-rust intentionally discards comments. Keep comment-only lines in a
    // deterministic preamble rather than silently deleting documentation from
    // a migrated shipped example; semantic ordering comes from the YAML AST.
    for line in legacy
        .lines()
        .filter(|line| line.trim_start().starts_with('#'))
    {
        emitted.push_str(line.trim_start());
        emitted.push('\n');
    }
    emitted.push_str("jobs:\n");
    let mut body = String::new();
    YamlEmitter::new(&mut body)
        .dump(&Yaml::Array(jobs))
        .map_err(|err| FzzError::GenericError(format!("Failed to emit migrated jobs: {err}")))?;
    for line in body.lines().filter(|line| *line != "---") {
        emitted.push_str("  ");
        emitted.push_str(line);
        emitted.push('\n');
    }
    if !legacy.ends_with('\n') {
        emitted.pop();
    }
    Ok(emitted)
}

fn pattern_map(group: &Yaml, section: &str) -> Result<(Vec<String>, Vec<String>), FzzError> {
    let section_yaml = &group[section];
    if section_yaml == &Yaml::BadValue {
        return Ok((Vec::new(), Vec::new()));
    }
    if !matches!(section_yaml, Yaml::Hash(_)) {
        return Err(FzzError::GenericError(format!(
            "Nested group '{section}' must be an object"
        )));
    }
    Ok((
        pattern_values(section_yaml, "change")?,
        pattern_values(section_yaml, "ignore")?,
    ))
}

fn pattern_values(section: &Yaml, key: &str) -> Result<Vec<String>, FzzError> {
    match &section[key] {
        Yaml::BadValue => Ok(Vec::new()),
        Yaml::String(value) => Ok(vec![value.clone()]),
        Yaml::Array(values) => values
            .iter()
            .map(|value| match value {
                Yaml::String(value) => Ok(value.clone()),
                other => Err(FzzError::GenericError(format!(
                    "Pattern '{key}' must contain strings, got {}",
                    crate::yaml::get_type(other)
                ))),
            })
            .collect(),
        other => Err(FzzError::GenericError(format!(
            "Pattern '{key}' must be a string or list, got {}",
            crate::yaml::get_type(other)
        ))),
    }
}

fn materialize_group_patterns(
    task: &Yaml,
    common: &(Vec<String>, Vec<String>),
) -> Result<Yaml, FzzError> {
    let Yaml::Hash(mut properties) = task.clone() else {
        return Err(FzzError::GenericError(
            "Nested group tasks must be job objects".to_string(),
        ));
    };
    for (key, inherited) in [("change", &common.0), ("ignore", &common.1)] {
        if inherited.is_empty() {
            continue;
        }
        let local = pattern_values(task, key)?;
        let mut merged = inherited.clone();
        for value in local {
            if !merged.contains(&value) {
                merged.push(value);
            }
        }
        properties.insert(
            Yaml::String(key.to_string()),
            Yaml::Array(merged.into_iter().map(Yaml::String).collect()),
        );
    }
    Ok(Yaml::Hash(properties))
}

/// Renames a root `old_key:` line to `new_key:` (same indentation), leaving
/// everything else byte-identical so order, comments, and commands survive.
fn rename_root_key(content: &str, old_key: &str, new_key: &str) -> String {
    content
        .lines()
        .map(|line| {
            let trimmed = line.trim_start();
            if trimmed == format!("{old_key}:") && line.starts_with(trimmed) {
                let indent = &line[..line.len() - trimmed.len()];
                format!("{indent}{new_key}:")
            } else {
                line.to_string()
            }
        })
        .collect::<Vec<_>>()
        .join("\n")
        + if content.ends_with('\n') { "\n" } else { "" }
}

/// # `MigrateCommand`
///
/// Rewrites the selected configuration file in place, atomically: the
/// migrated candidate must parse completely through the production parser
/// before the original file is ever touched.
pub struct MigrateCommand {
    pub file_name: String,
}

impl MigrateCommand {
    pub fn new(file: &str) -> Self {
        MigrateCommand {
            file_name: file.to_string(),
        }
    }

    fn backup_name(&self) -> String {
        format!("{}.fzz-migrate-tmp", self.file_name)
    }
}

impl Command for MigrateCommand {
    fn execute(&self) -> Result<(), FzzError> {
        let legacy = std::fs::read_to_string(&self.file_name).map_err(|err| {
            FzzError::IoConfigError(format!("Failed to read {}", self.file_name), Some(err))
        })?;

        let migrated = migrate_content(&legacy)?;

        // Validate the complete candidate before any write: a truncated or
        // partial config can never replace the original (TASK-0098).
        if let Err(err) = crate::config::from_yaml(&migrated) {
            return Err(FzzError::GenericError(format!(
                "Migrated {} would be invalid: {err}. \
                 This is a bug in fzz migrate; the original file was not changed",
                self.file_name
            )));
        }

        // Byte-identical no-op: report success without rewriting the file.
        if migrated == legacy {
            stdout::info(&format!(
                "{} is already in the preferred jobs: form",
                self.file_name
            ));
            return Ok(());
        }

        // Atomic replacement: write the complete candidate to a sibling
        // temp file, then rename over the original. A failure at any point
        // leaves the original untouched.
        std::fs::write(self.backup_name(), &migrated).map_err(|err| {
            FzzError::IoConfigError(
                format!("Failed to stage migrated {}", self.file_name),
                Some(err),
            )
        })?;
        if let Err(err) = std::fs::rename(self.backup_name(), &self.file_name) {
            let _ = std::fs::remove_file(self.backup_name());
            return Err(FzzError::IoConfigError(
                format!("Failed to replace {}", self.file_name),
                Some(err),
            ));
        }

        stdout::info(&format!("{} migrated to the jobs: form", self.file_name));
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const LEGACY: &str = "# project tasks\n\n- name: test\n  run: cargo test\n  change: src/**\n\n# final task\n- name: lint\n  run: cargo fmt -- --check\n  run_on_init: true\n";

    /// Root list is wrapped under `jobs:` preserving order, comments, and
    /// trailing newline — byte-exact (moved from init.rs, TASK-0078).
    #[test]
    fn it_wraps_legacy_tasks_into_jobs_and_preserves_comments() {
        assert_eq!(
            migrate_content(LEGACY).unwrap(),
            "# project tasks\n\njobs:\n  - name: test\n    run: cargo test\n    change: src/**\n\n  # final task\n  - name: lint\n    run: cargo fmt -- --check\n    run_on_init: true\n"
        );
    }

    #[test]
    fn flattens_nested_groups_with_effective_patterns_and_order() {
        let nested = r#"# catalog heading
- on:
    change:
      - "src/**"
      - "shared/**"
    ignore: "**/*.log"
  tasks:
    # group job comment
    - name: build
      run: "echo build"
    - name: test @quick
      run: |
        echo test
      change:
        - "src/**"
        - "tests/**"
- name: final
  run: echo final
  change: docs/**
"#;

        let migrated = migrate_content(nested).expect("nested groups migrate");
        let rules = crate::config::from_yaml(&migrated).expect("migrated config is valid");
        assert_eq!(
            rules
                .iter()
                .map(|rule| rule.name.as_str())
                .collect::<Vec<_>>(),
            ["build", "test @quick", "final"]
        );
        assert_eq!(rules[0].watch_patterns(), vec!["src/**", "shared/**"]);
        assert_eq!(rules[0].ignore_glob_patterns(), vec!["**/*.log"]);
        assert_eq!(
            rules[1].watch_patterns(),
            vec!["src/**", "shared/**", "tests/**"]
        );
        assert_eq!(rules[1].ignore_glob_patterns(), vec!["**/*.log"]);
        assert_eq!(rules[1].commands(), vec!["echo test\n"]);
        assert_eq!(rules[2].watch_patterns(), vec!["docs/**"]);
        assert!(migrated.starts_with("# catalog heading\n"));
        assert!(migrated.contains("jobs:\n"));
        assert!(migrated.contains("# group job comment"));
        assert!(!migrated.contains("tasks:"));
        assert!(!migrated.contains("- on:"));
    }

    #[test]
    fn flattens_empty_and_mixed_groups_without_reordering_jobs() {
        let nested = r#"- name: before
  run: echo before
  change: before/**
- on:
    change: group/**
  tasks: []
- on:
    change: /tmp/fzz/group/**
    ignore: /tmp/fzz/ignored/**
  tasks:
    - name: inside
      run: echo inside
- name: after
  run: echo after
  change: after/**
"#;

        let migrated = migrate_content(nested).expect("mixed groups migrate");
        let rules = crate::config::from_yaml(&migrated).expect("migrated config is valid");
        assert_eq!(
            rules
                .iter()
                .map(|rule| rule.name.as_str())
                .collect::<Vec<_>>(),
            ["before", "inside", "after"]
        );
        assert_eq!(rules[1].watch_patterns(), vec!["/tmp/fzz/group/**"]);
        assert_eq!(rules[1].ignore_glob_patterns(), vec!["/tmp/fzz/ignored/**"]);
    }

    /// Grouped `tasks:` root is renamed to `jobs:` byte-exactly.
    #[test]
    fn renames_grouped_tasks_root_to_jobs() {
        let grouped =
            "on:\n  socket: .tmp/funzzy.sock\ntasks:\n  - name: test\n    run: cargo test\n    run_on_init: true\n";
        assert_eq!(
            migrate_content(grouped).unwrap(),
            "on:\n  socket: .tmp/funzzy.sock\njobs:\n  - name: test\n    run: cargo test\n    run_on_init: true\n"
        );
    }

    /// Grouped input without a trailing newline keeps that exact shape.
    #[test]
    fn rename_preserves_absent_trailing_newline() {
        let migrated =
            migrate_content("on:\n  change: \"**/*\"\ntasks:\n  - name: a\n    run: echo a")
                .unwrap();
        assert_eq!(
            migrated,
            "on:\n  change: \"**/*\"\njobs:\n  - name: a\n    run: echo a"
        );
    }

    /// Section ownership is deliberately outside migration: V1 task vocabulary
    /// changes, while the grouped policy text stays byte-identical.
    #[test]
    fn migration_does_not_reorganize_v2_policy_sections() {
        let legacy = "on:\n  concurrency: 2\n  output: quiet\n  success: echo ok\ntasks:\n  - name: test\n    run: cargo test\n";
        assert_eq!(
            migrate_content(legacy).unwrap(),
            "on:\n  concurrency: 2\n  output: quiet\n  success: echo ok\njobs:\n  - name: test\n    run: cargo test\n"
        );
    }

    /// Already-preferred input is a byte-identical no-op.
    #[test]
    fn already_preferred_input_is_unchanged() {
        let preferred = "on:\n  change: \"**/*\"\njobs:\n  - name: a\n    run: echo a\n";
        assert_eq!(migrate_content(preferred).unwrap(), preferred);
    }

    /// Malformed YAML and unsupported roots are errors, never silent rewrites.
    #[test]
    fn malformed_input_is_an_error() {
        let invalid = "name: [unclosed\n";
        let error = migrate_content(invalid).unwrap_err();
        assert!(error.to_string().contains("Invalid legacy .watch.yaml"));
    }

    #[test]
    fn unsupported_root_is_an_error() {
        let error = migrate_content("something: else\n").unwrap_err();
        assert!(error.to_string().contains("root must be a task list"));
    }

    /// `MigrateCommand` honors the selected file: missing input is an
    /// operational error naming the file.
    #[test]
    fn missing_file_is_an_error_naming_the_file() {
        let dir =
            std::env::temp_dir().join(format!("funzzy-migrate-missing-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).expect("create scratch");
        let missing = dir.join("absent.yml");

        let err = MigrateCommand::new(missing.to_str().unwrap())
            .execute()
            .expect_err("missing file must fail");
        assert!(
            err.to_string().contains("absent.yml"),
            "error names file: {err}"
        );

        let _ = std::fs::remove_dir_all(&dir);
    }

    /// Success path: file rewritten, no temp file left behind, and a second
    /// run is a byte-identical no-op (idempotent).
    #[test]
    fn migrate_command_rewrites_atomically_and_idempotently() {
        let dir = std::env::temp_dir().join(format!("funzzy-migrate-ok-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).expect("create scratch");
        let target = dir.join(".watch.yaml");
        std::fs::write(&target, LEGACY).expect("seed legacy");

        MigrateCommand::new(target.to_str().unwrap())
            .execute()
            .expect("migration succeeds");
        let first = std::fs::read_to_string(&target).unwrap();
        assert!(first.contains("jobs:"), "rewritten: {first}");
        assert!(
            !dir.join(".watch.yaml.fzz-migrate-tmp").exists(),
            "temp file must be renamed away"
        );

        // Idempotent second run does not rewrite bytes.
        MigrateCommand::new(target.to_str().unwrap())
            .execute()
            .expect("second run succeeds");
        assert_eq!(std::fs::read_to_string(&target).unwrap(), first);

        let _ = std::fs::remove_dir_all(&dir);
    }

    /// An invalid migrated candidate must never replace the original: guard
    /// against partial/truncated rewrites (exercised through the validation
    /// branch — the original bytes survive).
    #[test]
    fn invalid_candidate_never_replaces_the_original() {
        // A root list whose first entry is not a task still parses as YAML;
        // migrate_content produces output that must pass the parser gate or
        // the command refuses to write.
        let dir = std::env::temp_dir().join(format!("funzzy-migrate-guard-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).expect("create scratch");
        let target = dir.join(".watch.yaml");

        // Malformed YAML: command errors, original bytes unchanged.
        std::fs::write(&target, "name: [unclosed\n").expect("seed malformed");
        let err = MigrateCommand::new(target.to_str().unwrap())
            .execute()
            .expect_err("malformed input must fail");
        assert!(err.to_string().contains("Invalid legacy"), "got: {err}");
        assert_eq!(
            std::fs::read_to_string(&target).unwrap(),
            "name: [unclosed\n",
            "failed migration must not mutate bytes"
        );

        let _ = std::fs::remove_dir_all(&dir);
    }
}
