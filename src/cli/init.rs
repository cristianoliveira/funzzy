use crate::cli::Command;
use crate::errors::FzzError;

use crate::option_catalog::{self, OptionSpec};
use crate::stdout;
use std::fs::{self, File};
use std::io::Write;
use yaml_rust::{Yaml, YamlLoader};

/// One comment line for a catalog property: purpose plus default and allowed
/// values when the catalog carries them (INIT-TEMPLATE-CONTRACT §5).
fn comment_for(spec: &OptionSpec, indent: &str) -> String {
    let help = spec.help.trim_end_matches(['.', ' ']);
    let mut line = format!("{indent}# {}: {help}", spec.name);
    if let Some(default) = spec.default {
        line.push_str(&format!(" (default: {default})"));
    }
    if let Some(values) = spec.values {
        line.push_str(&format!(" values: {values}"));
    }
    line
}

/// Catalog lookup scoped to an owner (on.change and job.change have
/// different help text).
fn find_in(owner: option_catalog::Owner, name: &str) -> Option<&'static OptionSpec> {
    match owner {
        option_catalog::Owner::On => option_catalog::on_specs(),
        option_catalog::Owner::Job => option_catalog::job_specs(),
        option_catalog::Owner::Root => option_catalog::root_specs(),
    }
    .iter()
    .find(|s| s.name == name)
}

/// Renders the comprehensive commented `.watch.yaml` starter (TASK-0093/0094):
/// a small active setup plus every supported optional property documented as a
/// comment near its owning section. Comments come from the canonical option
/// catalog, so schema, parser, and template share one metadata owner. Output
/// is deterministic bytes with no terminal-width, environment, repository, or
/// network dependence (INIT-TEMPLATE-CONTRACT §8).
///
/// Active part stays behaviorally equivalent to the historical echo/ls
/// starter: hello `echo` with `run_on_init`, a harmless `ls -a` file-change
/// example, and an active control socket.
pub fn render_init_template() -> String {
    let mut out = String::new();

    out.push_str(
        "## Funzzy events file — .watch.yaml\n\
         # Comprehensive commented starter: small active setup, every supported\n\
         # option documented in comments. Full reference: `fzz config schema`.\n\
         #\n\
         # Next commands:\n\
         #   fzz check                  validate this file (no watcher)\n\
         #   fzz list                   show configured jobs\n\
         #   fzz run <name>             run one job once\n\
         #   fzz / fzz watch            start watching\n\
         #   fzz control status         talk to the running watcher\n\
         #   fzz config example minimal tiny machine-copyable starter\n\
         \n\
         on:\n",
    );

    // `on:` block: every optional property commented with help + example.
    // `socket` is active (control surface is part of the starter), so its
    // example is the live key instead of a comment.
    for spec in option_catalog::on_specs() {
        out.push_str(&comment_for(spec, "  "));
        out.push('\n');
        if spec.name == "socket" {
            out.push_str("  socket: .tmp/funzzy/control.sock\n");
        } else {
            for line in spec.example {
                out.push_str(&format!("  # {line}\n"));
            }
        }
    }

    // `jobs:` block: commented reference job for job properties not already
    // shown actively (change/ignore/run_on_init appear above the active jobs).
    // Uncommenting the reference block yields a parser-valid extra job.
    out.push_str("\njobs:\n");
    out.push_str("  # Optional job properties — uncomment the reference job to activate:\n");
    out.push_str("  #\n");
    for spec in option_catalog::job_specs() {
        if matches!(spec.name, "change" | "ignore" | "run_on_init") {
            continue;
        }
        out.push_str(&comment_for(spec, "  "));
        out.push('\n');
    }
    out.push_str("  #\n");
    out.push_str("  # - name: reference\n");
    for spec in option_catalog::job_specs() {
        if matches!(spec.name, "name" | "run_on_init") {
            continue; // name is the block header; run_on_init is active below
        }
        for line in spec.example {
            out.push_str(&format!("  #   {line}\n"));
        }
    }

    // Active starter: behaviorally equivalent to the historical echo/ls
    // starter, with catalog-sourced help comments above the active keys.
    out.push_str("\n  - name: hello world\n");
    let init = find_in(option_catalog::Owner::Job, "run_on_init").expect("catalog run_on_init");
    out.push_str(&comment_for(init, "    "));
    out.push_str("\n    run_on_init: true\n");
    out.push_str("    run: echo \"Funzzy hello world! Next step, add rules into .watch.yaml\"\n");

    out.push_str("\n  - name: list files\n");
    let change = find_in(option_catalog::Owner::Job, "change").expect("catalog change");
    out.push_str(&comment_for(change, "    "));
    out.push_str("\n    change: '**/*.txt'\n");
    let ignore = find_in(option_catalog::Owner::Job, "ignore").expect("catalog ignore");
    out.push_str(&comment_for(ignore, "    "));
    out.push_str("\n    ignore: '**/*.log'\n");
    out.push_str("    run: 'ls -a'\n");

    out
}

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
                if let Err(err) = yaml.write_all(render_init_template().as_bytes()) {
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

#[cfg(test)]
mod renderer_tests {
    use super::render_init_template;
    use crate::option_catalog::{self, Owner};

    fn comment_lines(content: &str) -> Vec<String> {
        content
            .lines()
            .map(str::trim_start)
            .filter(|l| l.starts_with('#'))
            .map(str::to_string)
            .collect()
    }

    /// TASK-0094 criterion: deterministic bytes, stable ordering — two renders
    /// are byte-identical.
    #[test]
    fn render_is_deterministic() {
        let a = render_init_template();
        let b = render_init_template();
        assert_eq!(a, b);
    }

    /// The active part must stay immediately runnable: parses through the
    /// production parser and passes structural validation.
    #[test]
    fn rendered_template_parses_and_validates() {
        let content = render_init_template();
        let rules = crate::config::from_yaml(&content)
            .unwrap_or_else(|err| panic!("generated template must parse: {err:?}"));
        crate::rules::validate_rules(&rules)
            .unwrap_or_else(|err| panic!("generated template must validate: {err}"));
        assert!(
            rules.iter().any(|r| r.run_on_init()),
            "hello job must run on init"
        );
        assert!(
            rules.iter().any(|r| !r.watch_patterns().is_empty()),
            "file-change job must match"
        );
    }

    /// Active starter stays behaviorally equivalent to today's echo/ls
    /// starter: hello echo with run_on_init, harmless ls file-change example,
    /// active (uncommented) control socket.
    #[test]
    fn active_starter_is_equivalent_to_previous_default() {
        let content = render_init_template();
        assert!(content.contains("run_on_init: true"));
        assert!(content.contains("Funzzy hello world"));
        assert!(content.contains("ls -a"));
        assert!(content.contains("**/*.txt"));
        assert!(content.contains("**/*.log"));
        // Socket line is active, not commented.
        assert!(content.contains("  socket: .tmp/funzzy/control.sock"));
        assert!(!content.contains("# socket: .tmp/funzzy/control.sock"));
    }

    /// Every optional catalog property is documented as a comment near its
    /// owning section (INIT-TEMPLATE-CONTRACT §5).
    #[test]
    fn every_optional_catalog_property_appears_commented() {
        let content = render_init_template();
        let comments = comment_lines(&content);
        for spec in option_catalog::all_specs()
            .iter()
            .chain(option_catalog::job_specs())
        {
            if !option_catalog::is_optional(spec) {
                continue;
            }
            assert!(
                comments.iter().any(|c| c.contains(spec.name)),
                "comment for '{}' missing",
                spec.name
            );
        }
    }

    /// Uncommenting each documented scalar example yields parser-valid YAML
    /// (TASK-0094 criterion 6).
    #[test]
    fn catalog_examples_are_parser_valid_when_activated() {
        for spec in option_catalog::all_specs()
            .iter()
            .chain(option_catalog::job_specs())
        {
            if spec.required {
                continue; // name/run are structurally present in every config
            }
            let yaml = match spec.owner {
                Owner::On => {
                    let lines: Vec<String> =
                        spec.example.iter().map(|l| format!("  {l}")).collect();
                    format!(
                        "on:\n{}\njobs:\n  - name: a\n    run: echo a\n",
                        lines.join("\n")
                    )
                }
                Owner::Job => {
                    let lines: Vec<String> =
                        spec.example.iter().map(|l| format!("    {l}")).collect();
                    format!(
                        "jobs:\n  - name: a\n    run: echo a\n{}\n",
                        lines.join("\n")
                    )
                }
                Owner::Root => continue,
            };
            let result = crate::config::from_yaml(&yaml);
            assert!(
                result.is_ok(),
                "example for '{}' must be parser-valid:\n{yaml}\nerr: {:?}",
                spec.name,
                result.err()
            );
        }
    }

    /// Deterministic size budget ceiling (INIT-TEMPLATE-CONTRACT §8):
    /// ≤ 200 lines / ≤ 8 KiB.
    #[test]
    fn size_budget_ceiling_is_respected() {
        let content = render_init_template();
        assert!(
            content.lines().count() <= 200,
            "template exceeds line budget: {}",
            content.lines().count()
        );
        assert!(
            content.len() <= 8 * 1024,
            "template exceeds byte budget: {}",
            content.len()
        );
    }
}
