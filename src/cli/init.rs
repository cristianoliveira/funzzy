use crate::cli::Command;
use crate::errors::FzzError;

use crate::stdout;
use std::fs::{self, File};
use std::io::Write;
use yaml_rust::{Yaml, YamlLoader};

pub const DEFAULT_CONTENT: &str = "## Funzzy events file
# more details see: https://github.com/cristianoliveira/funzzy
#
# List here the jobs and the commands for this workflow
# then run `fzz` to start to work.

on:
  socket: .tmp/funzzy/control.sock

jobs:
  - name: hello world
    run: echo \"Funzzy hello world! Next step, add rules into .watch.yaml\"
    run_on_init: true

  - name: list files
    run: 'ls -a'
    change: '**/*.txt'
    ignore: '**/*.log'
";

/// # `InitCommand`
///
/// Creates a funzzy yaml boilerplate.
///
pub struct InitCommand {
    pub file_name: String,
    migrate: bool,
}

impl InitCommand {
    pub fn new(file: &str) -> Self {
        InitCommand {
            file_name: file.to_string(),
            migrate: false,
        }
    }

    pub fn migrate(file: &str) -> Self {
        InitCommand {
            file_name: file.to_string(),
            migrate: true,
        }
    }

    fn migrate_file(&self) -> Result<(), FzzError> {
        let legacy = fs::read_to_string(&self.file_name).map_err(|err| {
            FzzError::IoConfigError(
                "Failed to read legacy configuration file".to_string(),
                Some(err),
            )
        })?;
        let migrated = migrate_content(&legacy)?;

        fs::write(&self.file_name, migrated).map_err(|err| {
            FzzError::IoConfigError(
                "Failed to write migrated configuration file".to_string(),
                Some(err),
            )
        })?;

        stdout::info("Configuration file migrated successfully!");
        Ok(())
    }
}

/// Migrates an accepted legacy config to the preferred V2 `jobs:` format
/// (TASK-0075/0076): a root task list is wrapped into an ordered `jobs:`
/// list preserving declaration order and comments; a grouped `tasks:` root is
/// renamed to `jobs:`. Idempotent and atomic — never starts a watcher or
/// runs tasks.
fn migrate_content(legacy: &str) -> Result<String, FzzError> {
    let documents = YamlLoader::load_from_str(legacy).map_err(|err| {
        FzzError::InvalidConfigError(
            "Invalid legacy .watch.yaml".to_string(),
            Some(err),
            Some("Fix the YAML syntax before running `fzz init --migrate`".to_string()),
        )
    })?;

    if documents.len() != 1 {
        return Err(FzzError::GenericError(
            "Legacy .watch.yaml must contain exactly one YAML document".to_string(),
        ));
    }

    match documents.first() {
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

/// Renames a root `old_key:` line to `new_key:` (same indentation), leaving
/// everything else byte-identical so order, comments, and commands survive.
fn rename_root_key(content: &str, old_key: &str, new_key: &str) -> String {
    content
        .lines()
        .map(|line| {
            let trimmed = line.trim_start();
            if trimmed == format!("{}:", old_key) && line.starts_with(trimmed) {
                let indent = &line[..line.len() - trimmed.len()];
                format!("{}{}:", indent, new_key)
            } else {
                line.to_owned()
            }
        })
        .collect::<Vec<_>>()
        .join("\n")
        + if content.ends_with('\n') { "\n" } else { "" }
}

#[cfg(test)]
mod tests {
    use super::migrate_content;

    #[test]
    fn it_wraps_legacy_tasks_into_jobs_and_preserves_comments() {
        let legacy = "# project tasks\n\n- name: test\n  run: cargo test\n  change: src/**\n\n# final task\n- name: lint\n  run: cargo fmt -- --check\n  run_on_init: true\n";

        assert_eq!(
            migrate_content(legacy).unwrap(),
            "# project tasks\n\njobs:\n  - name: test\n    run: cargo test\n    change: src/**\n\n  # final task\n  - name: lint\n    run: cargo fmt -- --check\n    run_on_init: true\n"
        );
    }

    #[test]
    fn it_renames_grouped_tasks_root_to_jobs() {
        let current = "on:\n  socket: .tmp/funzzy.sock\ntasks:\n  - name: test\n    run: cargo test\n    run_on_init: true\n";

        assert_eq!(
            migrate_content(current).unwrap(),
            "on:\n  socket: .tmp/funzzy.sock\njobs:\n  - name: test\n    run: cargo test\n    run_on_init: true\n"
        );
    }

    #[test]
    fn it_treats_already_jobs_config_as_idempotent_noop() {
        let current = "on:\n  socket: .tmp/funzzy.sock\njobs:\n  - name: test\n    run: cargo test\n    run_on_init: true\n";

        // Idempotent (TASK-0078): already-preferred input returns unchanged.
        assert_eq!(migrate_content(current).unwrap(), current);
    }

    #[test]
    fn it_rejects_invalid_yaml_without_changing_it() {
        let invalid = "- name: test\n  run: [unterminated\n";

        let error = migrate_content(invalid).unwrap_err();

        assert!(error.to_string().contains("Invalid legacy .watch.yaml"));
    }
}

impl Command for InitCommand {
    fn execute(&self) -> Result<(), FzzError> {
        if self.migrate {
            return self.migrate_file();
        }

        if File::open(&self.file_name).is_ok() {
            return Err(FzzError::IoConfigError(
                "Configuration file already exists (.watch.yaml)".to_string(),
                None,
            ));
        }

        match File::create(&self.file_name) {
            Ok(mut yaml) => {
                if let Err(err) = yaml.write_all(DEFAULT_CONTENT.as_ref()) {
                    return Err(FzzError::IoConfigError(
                        "Failed to write into configuration file".to_string(),
                        Some(err),
                    ));
                }

                stdout::info("Configuration file created successfully! To start run `fzz`");

                Ok(())
            }
            Err(err) => Err(FzzError::IoConfigError(
                "Failed to create the configuration file".to_string(),
                Some(err),
            )),
        }
    }
}
