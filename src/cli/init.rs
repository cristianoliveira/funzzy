use crate::cli::Command;
use crate::errors::FzzError;

use crate::stdout;
use std::fs::{self, File};
use std::io::Write;
use yaml_rust::{Yaml, YamlLoader};

pub const DEFAULT_CONTENT: &str = "## Funzzy events file
# more details see: https://github.com/cristianoliveira/funzzy
#
# List here the tasks and the commands for this workflow
# then run `fzz` to start to work.

control:
  socket: .tmp/funzzy/control.sock

tasks:
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
        Some(document) if document["tasks"] != Yaml::BadValue => {
            return Err(FzzError::GenericError(
                "Configuration already uses the current .watch.yaml format".to_string(),
            ));
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

    migrated.push_str("tasks:\n");
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

#[cfg(test)]
mod tests {
    use super::migrate_content;

    #[test]
    fn it_wraps_legacy_tasks_and_preserves_comments() {
        let legacy = "# project tasks\n\n- name: test\n  run: cargo test\n  change: src/**\n\n# final task\n- name: lint\n  run: cargo fmt -- --check\n  run_on_init: true\n";

        assert_eq!(
            migrate_content(legacy).unwrap(),
            "# project tasks\n\ntasks:\n  - name: test\n    run: cargo test\n    change: src/**\n\n  # final task\n  - name: lint\n    run: cargo fmt -- --check\n    run_on_init: true\n"
        );
    }

    #[test]
    fn it_rejects_config_already_using_new_format() {
        let current = "control:\n  socket: .tmp/funzzy.sock\ntasks:\n  - name: test\n    run: cargo test\n    run_on_init: true\n";

        let error = migrate_content(current).unwrap_err();

        assert_eq!(
            error.to_string(),
            "Reason: Configuration already uses the current .watch.yaml format"
        );
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
