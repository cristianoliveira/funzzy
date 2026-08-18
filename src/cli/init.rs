use crate::cli::templates::Profile;
use crate::cli::Command;
use crate::errors::FzzError;

use crate::option_catalog::{self, OptionSpec};
use crate::stdout;
use std::fs::File;
use std::io::Write;

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
        option_catalog::Owner::Execution => option_catalog::execution_specs(),
        option_catalog::Owner::Hooks => option_catalog::hook_specs(),
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

    for (name, owner, specs) in [
        (
            "execution",
            option_catalog::Owner::Execution,
            option_catalog::execution_specs(),
        ),
        (
            "hooks",
            option_catalog::Owner::Hooks,
            option_catalog::hook_specs(),
        ),
    ] {
        out.push_str(&format!("\n{name}: {{}}\n"));
        for spec in specs {
            out.push_str(&comment_for(spec, "  "));
            out.push('\n');
            for line in spec.example {
                out.push_str(&format!("  # {line}\n"));
            }
        }
        debug_assert!(find_in(owner, specs[0].name).is_some());
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
    template: Profile,
}

impl InitCommand {
    pub fn new(file: &str, template: Profile) -> Self {
        InitCommand {
            file_name: file.to_string(),
            template,
        }
    }
}

impl Command for InitCommand {
    fn execute(&self) -> Result<(), FzzError> {
        if File::open(&self.file_name).is_ok() {
            return Err(FzzError::IoConfigError(
                "Configuration file already exists (.watch.yaml)".to_string(),
                None,
            ));
        }

        match File::create(&self.file_name) {
            Ok(mut yaml) => {
                if let Err(err) = yaml.write_all(self.template.render().as_bytes()) {
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
                Owner::Execution | Owner::Hooks => continue,
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

    /// Criterion 8 (TASK-0095): `fzz config example` profiles never inherit
    /// the human-commented init output (agent carries its own lean comments).
    /// `comprehensive` (TASK-0097) is by definition the init template, so it
    /// is excluded from the lean-profile set.
    #[test]
    fn example_profiles_do_not_inherit_init_header() {
        for profile in crate::cli::config::PROFILES
            .iter()
            .filter(|p| **p != "comprehensive")
        {
            let yaml = crate::cli::config::example_yaml(profile).expect("example renders");
            assert!(
                !yaml.contains("Comprehensive commented starter"),
                "{profile} must not inherit init header comments"
            );
            assert!(
                !yaml.contains("option documented in comments"),
                "{profile} must not inherit init header text"
            );
        }
    }
}

#[cfg(test)]
mod init_command_tests {
    use super::InitCommand;
    use crate::cli::templates::Profile;
    use crate::cli::Command;

    fn scratch(label: impl AsRef<str>) -> std::path::PathBuf {
        let label = label.as_ref();
        let dir = std::env::temp_dir().join(format!(
            "funzzy-init-custom-{}-{}",
            std::process::id(),
            label
        ));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).expect("create scratch");
        dir
    }

    /// TASK-0094/0095: `InitCommand` preserves custom-filename behavior and
    /// create-only semantics.
    #[test]
    fn custom_filename_creates_that_file_and_refuses_overwrite() {
        let dir = scratch("custom");
        let custom = dir.join("custom.yml");

        InitCommand::new(custom.to_str().unwrap(), Profile::Comprehensive)
            .execute()
            .expect("init must create custom.yml");
        assert!(custom.exists(), "custom.yml must be created");
        let bytes = std::fs::read(&custom).expect("read custom.yml");
        assert!(
            bytes == super::render_init_template().as_bytes(),
            "custom file must carry the deterministic template bytes"
        );

        // Create-only: second init refuses without mutating the file.
        let err = InitCommand::new(custom.to_str().unwrap(), Profile::Comprehensive).execute();
        assert!(err.is_err(), "second init on existing file must fail");
        assert_eq!(
            std::fs::read(&custom).expect("re-read custom.yml"),
            bytes,
            "refused init must not mutate"
        );

        let _ = std::fs::remove_dir_all(&dir);
    }

    /// TASK-0097: `fzz init --template P` writes bytes identical to
    /// `fzz config example P` stdout — one renderer, any destination.
    #[test]
    fn init_template_bytes_match_config_example_stdout_for_every_profile() {
        for profile in Profile::NAMES.map(|n| Profile::parse(n).unwrap()) {
            let dir = scratch(profile.name());
            let target = dir.join(".watch.yaml");

            InitCommand::new(target.to_str().unwrap(), profile)
                .execute()
                .unwrap_or_else(|err| panic!("{} init must succeed: {err}", profile.name()));

            let written = std::fs::read(&target).expect("read generated config");
            let exported = crate::cli::templates::render_profile(profile.name())
                .unwrap_or_else(|err| panic!("{} example must render: {err}", profile.name()));
            assert_eq!(
                written,
                exported.as_bytes(),
                "{}: init bytes must equal config example stdout",
                profile.name()
            );

            let _ = std::fs::remove_dir_all(&dir);
        }
    }

    /// Create-only refusal fires for every profile and never mutates the
    /// existing file (INIT-TEMPLATE-CONTRACT §1a).
    #[test]
    fn refusal_leaves_existing_bytes_unchanged_for_every_profile() {
        for profile in Profile::NAMES.map(|n| Profile::parse(n).unwrap()) {
            let dir = scratch(format!("refuse-{}", profile.name()));
            let target = dir.join(".watch.yaml");
            std::fs::write(&target, "# existing bytes\n").expect("seed existing config");

            let err = InitCommand::new(target.to_str().unwrap(), profile).execute();
            assert!(
                err.is_err(),
                "{}: existing file must be refused",
                profile.name()
            );
            assert_eq!(
                std::fs::read(&target).expect("re-read"),
                b"# existing bytes\n",
                "{}: refused init must not mutate bytes",
                profile.name()
            );

            let _ = std::fs::remove_dir_all(&dir);
        }
    }
}
