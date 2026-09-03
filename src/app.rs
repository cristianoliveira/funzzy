//! Application entry point: CLI parsing, command dispatch, and watch startup.
//!
//! This module is the canonical home of executable behavior. `src/main.rs`
//! is a thin process adapter that calls [`run`]; integration tests exercise
//! the same modules the binaries use.

use crate::arguments::{Action, Arguments, OnBusy};
use crate::cli;
use crate::cli::*;
use crate::duration_recorder::DurationRecorder;
use crate::duration_store::{state_file_path, DurationStore, STATE_SCHEMA_VERSION};
use crate::errors;
use crate::errors::FzzError;
use crate::reload_session::{ReloadSession, ReloadSettings};
use crate::watches::Watches;
use crate::{config, diagnostics, environment, logging, rules, stdout};

use nix::sys::signal::Signal;
use std::io::prelude::*;
use std::io::{self, IsTerminal};
use std::process;
use std::sync::mpsc;
use std::sync::mpsc::TryRecvError;
use std::sync::Arc;

/// Runs the application: parse arguments, choose the execution path, and
/// start the watcher or exit with a message.
pub fn run() {
    let args = Arguments::parse();

    // Diagnostics (TASK-0023): one process-wide sink gated on the verbose
    // flag; records render identically to terminal and log file.
    diagnostics::init(args.verbose);

    if args.log_truncate_on_change && args.log_file.is_none() {
        stdout::failure(
            "`--log-truncate-on-change` requires `--log-file`",
            "Provide a log file path before enabling truncation.".to_string(),
        );
    }

    if let Some(ref log_file) = args.log_file {
        if log_file.trim().is_empty() {
            stdout::failure("Invalid log file path", "Path cannot be empty".to_string());
        }

        let log_path = std::path::PathBuf::from(log_file);
        if let Some(parent) = log_path.parent() {
            if !parent.as_os_str().is_empty() && !parent.exists() {
                stdout::failure(
                    "Failed to prepare log file",
                    format!("directory does not exist: {}", parent.display()),
                );
            }
        }

        logging::init(log_path.clone())
            .unwrap_or_else(|err| stdout::failure("Failed to prepare log file", err.to_string()));

        stdout::info(&format!("Logging output to {}", log_path.display()));
    }

    // NDJSON run-event stream (TASK-0039): opened once, shared by every
    // executor sink in this process; None keeps behavior byte-identical.
    let event_stream = match args.events_file.as_deref() {
        Some(path) if !path.trim().is_empty() => {
            let stream = crate::event_stream::EventStream::open(std::path::Path::new(path))
                .unwrap_or_else(|err| {
                    stdout::failure("Failed to open run event stream", err.to_string())
                });
            stdout::info(&format!("Appending run events to {}", path));
            Some(Arc::new(stream))
        }
        _ => None,
    };

    // Resolve the workspace root once and anchor all watch planning,
    // config discovery, and command template preparation to it.
    let workspace_root = std::env::current_dir().expect("Failed to get current directory");

    match args.action {
        // Commands
        Action::Check => check_config(&args.config),
        Action::Completions { shell } => {
            let mut cmd = crate::arguments::command();
            let name = cmd.get_name().to_string();
            let generator = match shell.as_str() {
                "bash" => clap_complete::Shell::Bash,
                "zsh" => clap_complete::Shell::Zsh,
                "fish" => clap_complete::Shell::Fish,
                "elvish" => clap_complete::Shell::Elvish,
                "powershell" => clap_complete::Shell::PowerShell,
                other => {
                    stdout::failure(
                        "Invalid shell",
                        format!("expected bash, zsh, fish, elvish, or powershell, got {other}"),
                    );
                }
            };
            clap_complete::generate(generator, &mut cmd, name, &mut std::io::stdout());
        }
        Action::Config {
            schema_section,
            example_profile,
            format,
        } => {
            let result = crate::cli::config::execute_config(
                schema_section.flatten(),
                example_profile,
                format,
            );
            if let Err(err) = result {
                stdout::failure("config command failed", err.to_string());
            }
        }
        Action::Init { template } => {
            execute(InitCommand::new(cli::watch::DEFAULT_FILENAME, template))
        }
        Action::Migrate => {
            let config_path = args
                .config
                .clone()
                .unwrap_or_else(|| cli::watch::DEFAULT_FILENAME.to_string());
            if let Err(err) = MigrateCommand::new(&config_path).execute() {
                stdout::failure_to_stderr(
                    &format!("Failed to migrate {config_path}"),
                    err.to_string(),
                );
            }
        }
        Action::Control {
            action,
            socket,
            format,
        } => {
            let command = ControlCommand::with_format(
                action,
                socket,
                args.control_socket.clone(),
                args.config.clone(),
                format,
            );
            execute(command);
        }

        Action::Watch {
            target: ref wanted,
            exclude: ref exclusions,
            no_services,
        } => {
            let rules = load_rules(&args.config);
            let concurrency = effective_concurrency(&args, &args.config);
            let debounce = load_debounce(&args.config);
            let backend = load_watch_backend(&args.config);
            if let Err(err) = rules::validate_rules(&rules) {
                stdout::failure("Invalid config file.", err);
            }
            let watches = Watches::with_root_and_concurrency(
                rules.clone(),
                workspace_root.clone(),
                concurrency,
            )
            .with_debounce(debounce)
            .with_backend(backend)
            .with_gitignore(load_respect_gitignore(&args.config))
            .with_recovery_policy(effective_recovery_policy(&args, &args.config))
            .with_recovery_timeout(load_recovery_timeout(&args.config))
            .with_hooks(load_hooks(&args.config))
            .with_session_hooks(load_session_hooks(&args.config));
            // TASK-0092: resolve the config-declared control socket BEFORE
            // freezing the initial revision so the startup revision's semantic
            // surface matches every reload candidate (which always carries
            // `on.socket`). Otherwise a config-declared socket makes every
            // valid save — even a formatting-only rewrite — look like a
            // semantic change and commit a new revision.
            let control_socket = args
                .control_socket
                .clone()
                .or_else(|| config_control_socket(&args.config, &workspace_root));
            // TASK-0089: freeze the initial immutable revision before any
            // plan is created; reload (TASK-0090) observes candidates through
            // the same tracker and only commits on semantic change.
            let watches = {
                let mut tracker = crate::config_revision::RevisionTracker::new();
                let runtime = crate::config_revision::RuntimeConfig::capture(
                    workspace_root.clone(),
                    rules.clone(),
                    concurrency,
                    debounce,
                    backend,
                    load_respect_gitignore(&args.config),
                    effective_recovery_policy(&args, &args.config),
                    load_recovery_timeout(&args.config),
                    load_hooks(&args.config),
                    load_session_hooks(&args.config),
                    control_socket.as_deref().map(std::path::PathBuf::from),
                );
                match tracker.observe(&runtime) {
                    crate::config_revision::RevisionDecision::New(revision) => {
                        watches.with_revision(revision)
                    }
                    crate::config_revision::RevisionDecision::NoOp => watches,
                }
            };
            match watches.select_target_with_exclusions(wanted.as_deref(), exclusions, no_services)
            {
                Ok(Some(selected)) => execute_watch_command(selected, args, event_stream.clone()),
                Ok(None) => {
                    let target = wanted.as_deref().unwrap_or_default();
                    stdout::failure(
                        &format!("No target found for '{}'", target),
                        rules::available_targets(&rules),
                    )
                }
                Err(error) => {
                    stdout::usage_failure("Cannot apply watch exclusions", error.to_string())
                }
            }
        }
        Action::List => {
            let rules = load_rules(&args.config);
            if let Err(err) = rules::validate_rules(&rules) {
                stdout::failure("Invalid config file.", err);
            }
            stdout::info(&rules::available_targets(&rules));
        }
        Action::Run { ref target } => {
            let rules = load_rules(&args.config);
            if let Err(err) = rules::validate_rules(&rules) {
                stdout::failure("Invalid config file.", err);
            }
            let concurrency = effective_concurrency(&args, &args.config);
            let debounce = load_debounce(&args.config);
            let watches = Watches::with_root_and_concurrency(
                rules.clone(),
                workspace_root.clone(),
                concurrency,
            )
            .with_debounce(debounce)
            .with_recovery_policy(effective_recovery_policy(&args, &args.config))
            .with_recovery_timeout(load_recovery_timeout(&args.config));
            let plan = match watches.run_target_plan(target) {
                Ok(plan) => plan,
                Err(crate::watches::RunTargetError::Missing(_)) => stdout::failure(
                    &format!("No target found for '{}'", target),
                    rules::available_targets(&rules),
                ),
                Err(error) => stdout::failure("Cannot run target", error.to_string()),
            };

            let shutdown = install_shutdown_signal_handler(None);
            let fail_fast = args.fail_fast || environment::is_enabled("FUNZZY_BAIL");
            let command = RunCommand::with_recorder_and_events(
                workspace_root.clone(),
                args.verbose,
                fail_fast,
                concurrency,
                Some(Arc::new(DurationRecorder::new(DurationStore::new(
                    state_file_path(
                        &std::fs::canonicalize(&workspace_root)
                            .unwrap_or_else(|_| workspace_root.clone()),
                        STATE_SCHEMA_VERSION,
                    ),
                )))),
                event_stream.clone(),
            )
            .with_hooks(load_hooks(&args.config))
            .with_recovery_policy(effective_recovery_policy(&args, &args.config))
            .with_recovery_timeout(load_recovery_timeout(&args.config))
            .with_recovery_approval(Arc::new(crate::approval::TtyRecoveryApproval));
            let result = command.execute(plan, target);
            let signal_exit = shutdown.load(std::sync::atomic::Ordering::SeqCst);
            if signal_exit != 0 {
                process::exit(signal_exit);
            }
            match result {
                Ok(true) => {}
                Ok(false) => process::exit(1),
                Err(error) => stdout::failure("Configured run failed", error),
            }
        }
        Action::Explain { ref path } => {
            let rules = load_rules(&args.config);
            if let Err(err) = rules::validate_rules(&rules) {
                stdout::failure("Invalid config file.", err);
            }
            let watches = Watches::with_root_and_concurrency(
                rules.clone(),
                workspace_root.clone(),
                effective_concurrency(&args, &args.config),
            )
            .with_debounce(load_debounce(&args.config))
            .with_backend(load_watch_backend(&args.config))
            .with_gitignore(load_respect_gitignore(&args.config))
            .with_recovery_policy(effective_recovery_policy(&args, &args.config))
            .with_recovery_timeout(load_recovery_timeout(&args.config))
            .with_hooks(load_hooks(&args.config));
            let result = watches.explain(path);
            let facts = crate::watches::ExplainFacts {
                concurrency: watches.concurrency(),
                debounce: watches.debounce(),
            };
            stdout::info(&explain_output(path, &result, &facts, &watches));
        }

        // Ad-hoc command provided via `fzz exec -- PROGRAM ARG...`. The argv
        // is preserved end to end: it is never joined and re-parsed through a
        // shell. Shell operators only work when the caller explicitly invokes
        // a shell (e.g. `fzz exec -- sh -c '...'`).
        Action::Exec { command: ref cmd } => {
            match from_stdin() {
                Ok(StdinRead::NoPipe) => {
                    // No stdin and no config -> help and exit 1
                    println!("{}", Arguments::help_text());
                    process::exit(1);
                }
                Ok(StdinRead::PipeEmpty) => {
                    stdout::failure("No files provided via stdin.", "Provide a list of files or directories via stdin, e.g., `find . | fzz exec -- echo {{filepath}}`.".to_string());
                }
                Ok(StdinRead::Data(content)) => {
                    let patterns = match config::extract_paths(content) {
                        Ok(patterns) => patterns,
                        Err(err) => {
                            stdout::failure("Failed to get rules from stdin", err.to_string())
                        }
                    };

                    let watch_rules = match config::from_argv(patterns, cmd.clone()) {
                        Ok(rules) => {
                            stdout::info(&format!(
                                "watching patterns\r{}",
                                rules[0].watch_patterns().join("\n")
                            ));
                            rules
                        }
                        Err(err) => {
                            stdout::failure("Failed to get rules from stdin", err.to_string())
                        }
                    };

                    if let Err(err) = rules::validate_rules(&watch_rules) {
                        stdout::failure("Invalid config file.", err);
                    }

                    execute_watch_command(
                        Watches::with_root(watch_rules, workspace_root),
                        args,
                        event_stream.clone(),
                    );
                }
                Err(err) => stdout::failure("Failed to read stdin", err.to_string()),
            };
        }
    }
}

/// Renders a deterministic human summary of which tasks a path matches.
/// Reuses the structured `ExplainResult`; no matching logic lives here.
fn explain_output(
    path: &str,
    result: &crate::watches::ExplainResult,
    facts: &crate::watches::ExplainFacts,
    watches: &Watches,
) -> String {
    let mut output = String::from("Explain path ");
    output.push_str(path);
    output.push('\n');

    if result.matched.is_empty() && result.ignored.is_empty() {
        output.push_str("  unmatched: no configured task watches this path\n");
        // Contract §8: for a future/missing path, name the subscription root
        // that will observe it (nearest existing ancestor), so coverage is
        // explicit instead of silent.
        let covering = watches.covering_roots(path);
        if !covering.is_empty() {
            output.push_str(&format!(
                "  covered by subscription root(s): {}\n",
                covering.join(", ")
            ));
        }
        append_manual_section(&mut output, result);
        return output;
    }

    append_manual_section(&mut output, result);

    if !result.matched.is_empty() {
        output.push_str("  matched:\n");
        for rule in &result.matched {
            output.push_str(&format!("    - {}\n", rule.name));
            output.push_str(&format!("        cwd: {}\n", rule.cwd));
            output.push_str(&format!(
                "        env keys: [{}]\n",
                rule.environment_keys.join(", ")
            ));
            if rule.recovery_available {
                output.push_str("        recovery: configured (approval required)\n");
            }
            for change in &rule.change_patterns {
                output.push_str(&format!("        change: {}\n", change));
            }
        }
    }

    if !result.ignored.is_empty() {
        output.push_str("  ignored:\n");
        for rule in &result.ignored {
            output.push_str(&format!("    - {}\n", rule.name));
            output.push_str(&format!("        cwd: {}\n", rule.cwd));
            output.push_str(&format!(
                "        env keys: [{}]\n",
                rule.environment_keys.join(", ")
            ));
            if rule.recovery_available {
                output.push_str("        recovery: configured (approval required)\n");
            }
            for change in &rule.change_patterns {
                output.push_str(&format!("        change: {}\n", change));
            }
            for ignore in &rule.ignore_patterns {
                output.push_str(&format!("        ignored by: {}\n", ignore));
            }
        }
    }

    // TASK-0034: the filtered execution topology — barriers and named group
    // occurrences as they would actually run. Order is declaration order;
    // completion order inside a group is intentionally unspecified.
    if !result.plan_stages.is_empty() {
        output.push_str("  plan:\n");
        for stage in &result.plan_stages {
            match stage {
                crate::watches::PlanStagePreview::Serial { task } => {
                    output.push_str(&format!("    - {}\n", task));
                }
                crate::watches::PlanStagePreview::Parallel { group, tasks } => {
                    output.push_str(&format!(
                        "    - [{}] (parallel group): {}\n",
                        group,
                        tasks.join(" || ")
                    ));
                }
            }
        }
    }

    // Execution facts (TASK-0034): effective concurrency and debounce.
    output.push_str(&format!("  concurrency: {}\n", facts.concurrency));
    output.push_str(&format!("  debounce: {:?}\n", facts.debounce));

    output
}

/// MANUAL-TRIGGER-CONTRACT §6: name manual jobs so a no-match is explained
/// rather than mysterious — they never match paths by design.
fn append_manual_section(output: &mut String, result: &crate::watches::ExplainResult) {
    if result.manual.is_empty() {
        return;
    }
    output.push_str("  manual (never match filesystem events; explicit run only):\n");
    for name in &result.manual {
        output.push_str(&format!("    - {}\n", name));
    }
}

fn load_rules(config: &Option<String>) -> Vec<rules::Rules> {
    match config.as_deref() {
        None => config::from_default_file_config().unwrap_or_else(|err| {
            stdout::failure("Failed to read default config file", err.to_string());
        }),
        Some(config_file) => match config::from_file(config_file) {
            Ok(rules) => rules,
            Err(err) => stdout::failure("Failed to read config file", err.to_string()),
        },
    }
}

/// Effective scheduler concurrency: `--sequential` forces exactly 1 (the
/// sequential debugging override, SEQUENTIAL-OVERRIDE-CONTRACT §2); otherwise
/// the configured value from config or available parallelism applies.
fn effective_concurrency(args: &Arguments, config_file: &Option<String>) -> usize {
    if args.sequential {
        1
    } else {
        load_concurrency(config_file)
    }
}

/// `fzz check`: side-effect-free config validation (TASK-0033). Loads the
/// same parser/validator the watcher uses, plus debounce/concurrency/path
/// checks. Never starts a watcher, executes a task, or opens a socket.
fn check_config(config_file: &Option<String>) {
    let config_path = config_file
        .clone()
        .unwrap_or_else(|| cli::watch::DEFAULT_FILENAME.to_string());
    let rules = match config::from_file(&config_path) {
        Ok(rules) => rules,
        Err(err) => stdout::failure("Invalid config file.", err.to_string()),
    };
    if let Err(err) = rules::validate_rules(&rules) {
        stdout::failure("Invalid config file.", err);
    }
    if let Err(err) = config::generation_hooks_from_file(&config_path) {
        stdout::failure("Invalid hooks config", err);
    }
    if let Err(err) = config::session_hooks_from_file(&config_path) {
        stdout::failure("Invalid watcher close hook.", err);
    }
    // Debounce and concurrency reuse the exact watch-time parsers.
    if let Some(debounce) = config::debounce_from_file(&config_path)
        .unwrap_or_else(|err| stdout::failure("Invalid debounce config", err))
    {
        stdout::info(&format!("debounce: {:?}", debounce));
    }
    let _recovery_policy = config::recovery_policy_from_file(&config_path)
        .unwrap_or_else(|err| stdout::failure("Invalid recovery policy config", err));
    let concurrency = config::concurrency_from_file(&config_path)
        .unwrap_or_else(|err| stdout::failure("Invalid concurrency config", err))
        .unwrap_or_else(|| {
            std::thread::available_parallelism()
                .map(|p| p.get())
                .unwrap_or(1)
        });

    // Filesystem metadata checks only (never registers watches).
    let mut missing_paths: Vec<String> = vec![];
    for rule in &rules {
        for pattern in rule.watch_patterns() {
            if let Some(literal) = literal_prefix(&pattern) {
                if !std::path::Path::new(&literal).exists() {
                    missing_paths.push(literal);
                }
            }
        }
    }
    let groups = rules
        .iter()
        .filter(|rule| rule.parallel().is_some())
        .count();

    if !missing_paths.is_empty() {
        missing_paths.sort();
        missing_paths.dedup();
        stdout::warn(&format!(
            "paths do not exist (may still match future files): {}",
            missing_paths.join(", ")
        ));
    }

    // SERVICE-LIFECYCLE-CONTRACT §5–§6: an init-only service (service: true,
    // run_on_init: true, empty effective change patterns) is legal but dies
    // on the first superseding generation and nothing re-includes it.
    // Actionable warning, never a rejection: the shape is valid and has a
    // legitimate split-instance use.
    let init_only_services: Vec<&str> = rules
        .iter()
        .filter(|rule| rule.service() && rule.run_on_init() && rule.watch_patterns().is_empty())
        .map(|rule| rule.name.as_str())
        .collect();
    if !init_only_services.is_empty() {
        stdout::warn(&format!(
            "init-only service(s) [{}] are not automatically re-included by unrelated replacement generations: the service is reaped when its generation is superseded. Add `change:` patterns so re-inclusion restarts it, or isolate it in a dedicated config instance",
            init_only_services.join(", ")
        ));
    }
    stdout::info(&format!(
        "config valid: {} job(s), {} in parallel group(s), concurrency {}",
        rules.len(),
        groups,
        concurrency
    ));
}

/// Longest literal prefix of a change glob before the first wildcard, or
/// None when the pattern is fully wildcard. Used for path-existence checks.
fn literal_prefix(pattern: &str) -> Option<String> {
    let end = pattern.find(['*', '?', '[']).unwrap_or(pattern.len());
    let prefix = pattern[..end].trim_end_matches('/');
    if prefix.is_empty() {
        None
    } else {
        Some(prefix.to_owned())
    }
}

/// The filesystem debounce window from `on.debounce` (TASK-0031); defaults
/// to the historical one second when absent or invalid (invalid values fail
/// loudly, they never silently change timing).
fn load_debounce(config_file: &Option<String>) -> std::time::Duration {
    let path = match config_file.as_deref() {
        Some(path) => Some(path.to_owned()),
        None if std::path::Path::new(cli::watch::DEFAULT_FILENAME).exists() => {
            Some(cli::watch::DEFAULT_FILENAME.to_owned())
        }
        None => {
            let yaml = cli::watch::DEFAULT_FILENAME.replace(".yaml", ".yml");
            std::path::Path::new(&yaml).exists().then_some(yaml)
        }
    };
    let Some(path) = path else {
        return std::time::Duration::from_millis(1000);
    };
    config::debounce_from_file(&path)
        .unwrap_or_else(|err| stdout::failure("Invalid debounce config", err))
        .unwrap_or_else(|| std::time::Duration::from_millis(1000))
}

/// The filesystem backend policy from `on.watch_backend` (TASK-0037);
/// defaults to auto (native first, poll fallback). Invalid values fail loudly.
fn load_watch_backend(config_file: &Option<String>) -> crate::watcher::WatchBackend {
    let path = match config_file.as_deref() {
        Some(path) => Some(path.to_owned()),
        None if std::path::Path::new(cli::watch::DEFAULT_FILENAME).exists() => {
            Some(cli::watch::DEFAULT_FILENAME.to_owned())
        }
        None => {
            let yaml = cli::watch::DEFAULT_FILENAME.replace(".yaml", ".yml");
            std::path::Path::new(&yaml).exists().then_some(yaml)
        }
    };
    let Some(path) = path else {
        return crate::watcher::WatchBackend::Auto;
    };
    config::watch_backend_from_file(&path)
        .unwrap_or_else(|err| stdout::failure("Invalid watch backend config", err))
        .unwrap_or(crate::watcher::WatchBackend::Auto)
}

/// The run-level terminal hooks from `on.success`/`on.failure` (TASK-0040).
fn load_hooks(config_file: &Option<String>) -> config::GenerationHooks {
    // failure_settle defaults to None for legacy callers
    let path = match config_file.as_deref() {
        Some(path) => Some(path.to_owned()),
        None if std::path::Path::new(cli::watch::DEFAULT_FILENAME).exists() => {
            Some(cli::watch::DEFAULT_FILENAME.to_owned())
        }
        None => {
            let yaml = cli::watch::DEFAULT_FILENAME.replace(".yaml", ".yml");
            std::path::Path::new(&yaml).exists().then_some(yaml)
        }
    };
    let Some(path) = path else {
        return config::GenerationHooks::default();
    };
    config::generation_hooks_from_file(&path)
        .unwrap_or_else(|err| stdout::failure("Invalid hooks config", err))
}

/// Watcher-session close hook from `on.close` (TASK-0101). Kept separate
/// from generation hooks so finite runners never receive it.
fn load_session_hooks(config_file: &Option<String>) -> config::SessionHooks {
    let path = match config_file.as_deref() {
        Some(path) => Some(path.to_owned()),
        None if std::path::Path::new(cli::watch::DEFAULT_FILENAME).exists() => {
            Some(cli::watch::DEFAULT_FILENAME.to_owned())
        }
        None => {
            let yaml = cli::watch::DEFAULT_FILENAME.replace(".yaml", ".yml");
            std::path::Path::new(&yaml).exists().then_some(yaml)
        }
    };
    let Some(path) = path else {
        return config::SessionHooks::default();
    };
    config::session_hooks_from_file(&path)
        .unwrap_or_else(|err| stdout::failure("Invalid session hooks config", err))
}

/// Whether `on.respect_gitignore` is enabled (TASK-0036); default false.
fn load_respect_gitignore(config_file: &Option<String>) -> bool {
    let path = match config_file.as_deref() {
        Some(path) => Some(path.to_owned()),
        None if std::path::Path::new(cli::watch::DEFAULT_FILENAME).exists() => {
            Some(cli::watch::DEFAULT_FILENAME.to_owned())
        }
        None => {
            let yaml = cli::watch::DEFAULT_FILENAME.replace(".yaml", ".yml");
            std::path::Path::new(&yaml).exists().then_some(yaml)
        }
    };
    let Some(path) = path else {
        return false;
    };
    config::respect_gitignore_from_file(&path)
        .unwrap_or_else(|err| stdout::failure("Invalid gitignore config", err))
}

fn effective_recovery_policy(
    args: &Arguments,
    config_file: &Option<String>,
) -> config::RecoveryPolicy {
    args.recovery_policy
        .unwrap_or_else(|| load_recovery_policy(config_file))
}

fn load_recovery_timeout(config_file: &Option<String>) -> std::time::Duration {
    let path = match config_file.as_deref() {
        Some(path) => Some(path.to_owned()),
        None if std::path::Path::new(cli::watch::DEFAULT_FILENAME).exists() => {
            Some(cli::watch::DEFAULT_FILENAME.to_owned())
        }
        None => {
            let yaml = cli::watch::DEFAULT_FILENAME.replace(".yaml", ".yml");
            std::path::Path::new(&yaml).exists().then_some(yaml)
        }
    };
    let Some(path) = path else {
        return std::time::Duration::from_secs(60);
    };
    config::recovery_timeout_from_yaml_with_default(
        &std::fs::read_to_string(&path).unwrap_or_default(),
        std::time::Duration::from_secs(60),
    )
    .unwrap_or_else(|err| stdout::failure("Invalid recovery timeout config", err))
}

fn load_recovery_policy(config_file: &Option<String>) -> config::RecoveryPolicy {
    let path = match config_file.as_deref() {
        Some(path) => Some(path.to_owned()),
        None if std::path::Path::new(cli::watch::DEFAULT_FILENAME).exists() => {
            Some(cli::watch::DEFAULT_FILENAME.to_owned())
        }
        None => {
            let yaml = cli::watch::DEFAULT_FILENAME.replace(".yaml", ".yml");
            std::path::Path::new(&yaml).exists().then_some(yaml)
        }
    };
    let Some(path) = path else {
        return config::RecoveryPolicy::Prompt;
    };
    config::recovery_policy_from_file(&path)
        .unwrap_or_else(|err| stdout::failure("Invalid recovery policy config", err))
}

fn load_concurrency(config_file: &Option<String>) -> usize {
    let default = std::thread::available_parallelism()
        .map(|parallelism| parallelism.get())
        .unwrap_or(1);
    let path = match config_file.as_deref() {
        Some(path) => Some(path.to_owned()),
        None if std::path::Path::new(cli::watch::DEFAULT_FILENAME).exists() => {
            Some(cli::watch::DEFAULT_FILENAME.to_owned())
        }
        None => {
            let yaml = cli::watch::DEFAULT_FILENAME.replace(".yaml", ".yml");
            std::path::Path::new(&yaml).exists().then_some(yaml)
        }
    };
    let Some(path) = path else {
        return default;
    };

    config::concurrency_from_file(&path)
        .unwrap_or_else(|err| stdout::failure("Invalid concurrency config", err))
        .unwrap_or(default)
}

/// The control socket declared by the selected config file (`on.socket`),
/// mirroring `execute_watch_command` path resolution (explicit path, else
/// `.watch.yaml`/`.watch.yml` under the workspace root). Missing config or
/// missing key yields `None`. Used by the Watch command so the initial
/// revision's semantic surface includes the same socket the reload
/// candidates carry (TASK-0092).
fn config_control_socket(config: &Option<String>, root: &std::path::Path) -> Option<String> {
    let possible = match config.as_deref() {
        None => vec![
            root.join(cli::watch::DEFAULT_FILENAME),
            root.join(cli::watch::DEFAULT_FILENAME.replace("yaml", "yml")),
        ],
        Some(config_file) => vec![std::path::PathBuf::from(config_file)],
    };
    possible
        .into_iter()
        .find(|path| path.exists())
        .and_then(|path| {
            config::control_socket_from_file(&path.to_string_lossy())
                .unwrap_or_else(|err| stdout::failure("Invalid control socket config", err))
        })
}

fn execute_watch_command(
    watches: Watches,
    mut args: Arguments,
    event_stream: Option<Arc<crate::event_stream::EventStream>>,
) {
    let (watch_target, watch_exclusions, watch_no_services) = match &args.action {
        Action::Watch {
            target,
            exclude,
            no_services,
        } => (target.clone(), exclude.clone(), *no_services),
        _ => (None, Vec::new(), false),
    };
    let startup_exclusion_note = if watch_exclusions.is_empty() && !watch_no_services {
        None
    } else {
        Some(format!(
            "exclusions={} no_services={}",
            if watch_exclusions.is_empty() {
                "none".to_owned()
            } else {
                watch_exclusions.join(",")
            },
            watch_no_services
        ))
    };

    // MANUAL-TRIGGER-CONTRACT §3.5: a watch selection containing only manual
    // jobs is a usage error unless the control socket is enabled (a
    // control-only watcher serving `fzz ctl run` is valid).
    if !watches.all_rules().iter().any(|rule| !rule.is_manual()) {
        let workspace_root = watches.root().to_path_buf();
        let socket = args
            .control_socket
            .clone()
            .or_else(|| config_control_socket(&args.config, &workspace_root));
        if socket.is_none() {
            stdout::failure(
                "Nothing to watch: every selected job is 'trigger: manual'.",
                "Manual jobs run only via `fzz run TARGET` / `fzz ctl run TARGET`. Enable the control socket (`on.socket`) to keep a control-only watcher, or select change-triggered jobs.".to_string(),
            );
        }
    }

    let possible_config_paths = match args.config.as_deref() {
        None => {
            let dir = watches.root();
            vec![
                dir.join(cli::watch::DEFAULT_FILENAME)
                    .to_str()
                    .unwrap()
                    .to_string(),
                dir.join(cli::watch::DEFAULT_FILENAME.replace("yaml", "yml"))
                    .to_str()
                    .unwrap()
                    .to_string(),
            ]
        }
        Some(config_file) => vec![config_file.to_string()],
    };

    let truncate_on_config_change = args.log_truncate_on_change;
    let debounce = load_debounce(&args.config);

    let config_file_paths = possible_config_paths
        .into_iter()
        .filter(|path| std::path::Path::new(path).exists())
        .collect::<Vec<String>>();

    if args.control_socket.is_none() {
        args.control_socket = config_control_socket(&args.config, watches.root());
    }

    // Composition root owns one shutdown coordinator for the whole ready
    // watcher session. Finite commands never receive it (TASK-0101).
    let shutdown = crate::shutdown::ShutdownCoordinator::system(
        watches.root().to_path_buf(),
        watches.session_hooks(),
        args.verbose,
    );
    shutdown.set_cleanup_paths(
        args.control_socket
            .clone()
            .map(std::path::PathBuf::from)
            .into_iter()
            .collect(),
    );

    // TASK-0088/0090: replace unconditional self-SIGTERM with a
    // validate-first branch. The reload thread watches the config file
    // paths (baseline mtime guard rejects stale/historical replays), reads
    // a stable candidate, validates it through the four gates, and then
    // either commits a hot reload (same process) or shuts down gracefully
    // with a nonzero exit (never leaves old config running silently).
    let shared = std::sync::Arc::new(std::sync::Mutex::new(watches.clone()));
    let coordinator =
        crate::reload_coordinator::ReloadCoordinator::new(std::sync::Arc::clone(&shared));
    // TASK-0092: track the active control socket so a fatal shutdown can
    // remove its file before exit (`process::exit` skips the server Drop).
    coordinator.set_active_socket(args.control_socket.clone().map(std::path::PathBuf::from));
    let reload_settings = ReloadSettings {
        config_file_paths: config_file_paths.clone(),
        debounce,
        truncate_on_config_change,
        current_socket: args.control_socket.clone(),
        target: watch_target,
        exclusions: watch_exclusions.clone(),
        no_services: watch_no_services,
    };
    let mut reload_session = ReloadSession::start(
        reload_settings,
        &watches,
        coordinator.clone(),
        std::sync::Arc::clone(&shutdown),
    );

    let verbose = args.verbose;
    let fail_fast = args.fail_fast || environment::is_enabled("FUNZZY_BAIL");
    let non_block = matches!(args.on_busy, OnBusy::Restart)
        || environment::is_enabled("FUNZZY_NON_BLOCK")
        || args.control_socket.is_some();

    let run_on_init = !args.no_run_on_init;
    if verbose {
        emit_startup_record(
            &watches,
            &config_file_paths,
            args.control_socket.as_deref(),
            args.log_file.as_deref(),
            run_on_init,
            non_block,
            startup_exclusion_note.as_deref(),
        );
    }
    // Both wait and restart watch modes share one signal notification and
    // shutdown coordinator. The self-pipe thread requests only; this normal
    // composition-root flow executes configured close work (TASK-0101).
    let _signal_exit = install_shutdown_signal_handler(Some(std::sync::Arc::clone(&shutdown)));
    let watch_result = if non_block {
        WatchNonBlockCommand::with_events(
            watches,
            verbose,
            fail_fast,
            run_on_init,
            args.control_socket.map(std::path::PathBuf::from),
            event_stream,
        )
        .with_reload(coordinator.clone())
        .with_reload_ready(reload_session.take_ready())
        .with_shutdown(std::sync::Arc::clone(&shutdown))
        .execute()
    } else {
        WatchCommand::with_events(watches, verbose, fail_fast, run_on_init, event_stream)
            .with_shutdown(std::sync::Arc::clone(&shutdown))
            .execute()
    };

    if let Err(err) = watch_result {
        shutdown.request(crate::shutdown::ShutdownReason::Operational {
            detail: err.to_string(),
            exit_code: 1,
        });
    } else if !shutdown.is_requested() {
        shutdown.request(crate::shutdown::ShutdownReason::Normal);
    }

    reload_session.join();
    shutdown.set_cleanup_paths(coordinator.socket_paths_to_cleanup());
    let completion = shutdown.finish();
    report_shutdown_completion(&completion);
    if completion.reason.exit_code() != 0 {
        process::exit(completion.reason.exit_code());
    }
}

fn report_shutdown_completion(completion: &crate::shutdown::ShutdownCompletion) {
    use crate::shutdown::CloseHookOutcome;
    let reason = completion.reason.label();
    let message = match &completion.hook {
        CloseHookOutcome::Failed(error) => {
            Some(format!("close hook failed during {reason}: {error}"))
        }
        CloseHookOutcome::TimedOut => Some(format!("close hook timed out during {reason}")),
        CloseHookOutcome::Cancelled => Some(format!("close hook cancelled during {reason}")),
        _ => None,
    };
    if let Some(message) = message {
        eprintln!("Funzzy warning: {message}");
        logging::log_line(&format!("Funzzy warning: {message}"));
    }
    diagnostics::debug(&diagnostics::Record {
        source: Some("close_hook"),
        decision: Some(match &completion.hook {
            CloseHookOutcome::Passed => "passed",
            CloseHookOutcome::Failed(_) => "failed",
            CloseHookOutcome::TimedOut => "timeout",
            CloseHookOutcome::Cancelled => "cancelled",
            CloseHookOutcome::NotConfigured | CloseHookOutcome::SkippedBeforeReady => "skipped",
        }),
        note: Some(format!("reason={reason}")),
        ..Default::default()
    });
}

/// One deterministic startup record (TASK-0023): config path, workspace
/// root, watch roots, task count, busy policy, run-on-init state, log
/// destination, control socket, and (when requested) invocation exclusions.
/// The summary is compact and never dumps the full configuration.
fn emit_startup_record(
    watches: &Watches,
    config_file_paths: &[String],
    control_socket: Option<&str>,
    log_file: Option<&str>,
    run_on_init: bool,
    non_block: bool,
    invocation_note: Option<&str>,
) {
    let config_path = config_file_paths
        .first()
        .cloned()
        .unwrap_or_else(|| "default".to_owned());
    let watch_roots = watches.paths_to_watch().unwrap_or_default().join(",");
    let policy = if non_block { "restart" } else { "wait" };
    let invocation_note = invocation_note
        .map(|note| format!(" {note}"))
        .unwrap_or_default();
    diagnostics::debug(&diagnostics::Record {
        source: Some("config"),
        decision: Some("startup"),
        path: Some(config_path),
        note: Some(format!(
            "workspace={} tasks={} policy={} run_on_init={} log={} socket={} watch_roots={}{}",
            watches.root().display(),
            watches.targets().len(),
            policy,
            run_on_init,
            log_file.unwrap_or("none"),
            control_socket.unwrap_or("none"),
            watch_roots,
            invocation_note,
        )),
        ..Default::default()
    });
}

/// Catches SIGINT and SIGTERM via a disposition-based handler plus
/// self-pipe and routes them through the shared process-group ownership path
/// (`process_owner::shutdown_all`) before exiting with the conventional
/// code (130/143), so descendants in their own groups are not orphaned.
///
/// Installed for non-block watch and finite local run because executor child
/// tasks lead separate process groups and need explicit signal forwarding.
/// A handler (not block+sigwait) is required: any thread with an unblocked
/// signal is an eligible delivery target, and library threads created before
/// installation keep empty masks, which made the old design die to the
/// default action without cleanup on loaded Linux systems.
fn install_shutdown_signal_handler(
    shutdown: Option<std::sync::Arc<crate::shutdown::ShutdownCoordinator>>,
) -> std::sync::Arc<std::sync::atomic::AtomicI32> {
    use nix::sys::signal::{sigaction, SaFlags, SigAction, SigHandler, SigSet};
    use nix::unistd;
    use std::sync::atomic::{AtomicI32, Ordering};
    use std::sync::Arc;

    /// Self-pipe write end handed to the signal handler (TASK-0030). The
    /// handler performs only the async-signal-safe write(2); all real work
    /// happens on a normal thread after the byte arrives.
    static WAKE_WRITE_FD: std::sync::OnceLock<std::os::fd::OwnedFd> = std::sync::OnceLock::new();

    extern "C" fn on_shutdown_signal(signal: nix::libc::c_int) {
        // Async-signal-safe: one best-effort byte; nothing else.
        if let Some(fd) = WAKE_WRITE_FD.get() {
            let _ = unistd::write(fd, &[signal as u8]);
        }
    }

    let exit_code = Arc::new(AtomicI32::new(0));
    let signal_exit_code = Arc::clone(&exit_code);
    let watcher_shutdown = shutdown;

    // A disposition-based handler is required: the previous block+sigwait
    // design silently depended on every thread keeping SIGINT/SIGTERM
    // blocked, but any library thread spawned with an empty mask (watcher,
    // control socket, executor helpers) becomes an eligible delivery
    // target, and a process-directed signal then runs the DEFAULT action
    // and kills funzzy without cleanup or a conventional exit code. A
    // handler is process-wide regardless of per-thread masks; the self-pipe
    // keeps the handler itself async-signal-safe.
    let (wake_read_fd, wake_write_fd) = unistd::pipe().expect("shutdown self-pipe");
    // Task children must not hold the shutdown pipe across exec; execve
    // already resets caught-signal dispositions, and CLOEXEC keeps the pipe
    // itself out of the children too.
    use nix::fcntl::{fcntl, FdFlag, F_SETFD};
    let _ = fcntl(&wake_read_fd, F_SETFD(FdFlag::FD_CLOEXEC));
    let _ = fcntl(&wake_write_fd, F_SETFD(FdFlag::FD_CLOEXEC));
    let _ = WAKE_WRITE_FD.set(wake_write_fd);
    let action = SigAction::new(
        SigHandler::Handler(on_shutdown_signal),
        SaFlags::SA_RESTART,
        SigSet::all(),
    );
    if let Err(err) = unsafe { sigaction(Signal::SIGINT, &action) } {
        stdout::error(&format!("failed to install SIGINT handler: {err:?}"));
    }
    if let Err(err) = unsafe { sigaction(Signal::SIGTERM, &action) } {
        stdout::error(&format!("failed to install SIGTERM handler: {err:?}"));
    }

    std::thread::spawn(move || {
        let mut signal_byte = [0u8; 1];
        loop {
            match unistd::read(&wake_read_fd, &mut signal_byte) {
                Ok(_) => {
                    let code = if signal_byte[0] == Signal::SIGINT as u8 {
                        130
                    } else {
                        143
                    };
                    let _ = signal_exit_code.compare_exchange(
                        0,
                        code,
                        Ordering::SeqCst,
                        Ordering::SeqCst,
                    );
                    if let Some(shutdown) = &watcher_shutdown {
                        shutdown.request(crate::shutdown::ShutdownReason::Signal {
                            name: if code == 130 { "SIGINT" } else { "SIGTERM" },
                            exit_code: code,
                        });
                    } else {
                        // Finite local run: no watcher close hook. Reap the
                        // owned child groups; main command flow observes the
                        // stored code and exits after execute returns.
                        let (signal, grace) = crate::process_owner::shutdown_policy();
                        let _ = crate::process_owner::shutdown_all(signal, grace, false);
                    }
                }
                Err(nix::errno::Errno::EINTR) => continue,
                Err(_) => return,
            }
        }
    });
    exit_code
}

fn execute<T: Command>(command: T) {
    if let Err(err) = command.execute() {
        stdout::failure("Command failed to execute", err.to_string());
    }
}

enum StdinRead {
    NoPipe,
    PipeEmpty,
    Data(String),
}

fn from_stdin() -> errors::Result<StdinRead> {
    let stdin = io::stdin();

    // Check if stdin is a tty (interactive) - no pipe
    if stdin.is_terminal() {
        // No pipe
        return Ok(StdinRead::NoPipe);
    }

    // There is a pipe, read with patience
    let (tx, rx) = mpsc::channel();
    let handle = std::thread::spawn(move || {
        let mut buffer = String::new();
        let result = stdin.lock().read_to_string(&mut buffer);
        tx.send((buffer, result)).unwrap();
    });

    // Give the thread a chance to run (especially for immediate data)
    std::thread::yield_now();

    // Check if data is already available without waiting
    match rx.try_recv() {
        Ok((buffer, result)) => {
            // Thread finished reading already (fast)
            let _ = handle.join();
            match result {
                Ok(bytes) => {
                    if bytes > 0 {
                        Ok(StdinRead::Data(buffer))
                    } else {
                        // EOF with no data
                        Ok(StdinRead::PipeEmpty)
                    }
                }
                Err(err) => Err(FzzError::IoStdinError(err.to_string(), None)),
            }
        }
        Err(TryRecvError::Empty) => {
            // No data yet, wait with grace period for first data
            // Configurable via FUNZZY_STDIN_TIMEOUT_MS environment variable (default 2000 ms)
            let grace_period = std::env::var("FUNZZY_STDIN_TIMEOUT_MS")
                .ok()
                .and_then(|v| v.parse::<u64>().ok())
                .map(std::time::Duration::from_millis)
                .unwrap_or_else(|| std::time::Duration::from_secs(2));
            match rx.recv_timeout(grace_period) {
                Ok((buffer, result)) => {
                    // Thread finished reading within grace period
                    let _ = handle.join();
                    match result {
                        Ok(bytes) => {
                            if bytes > 0 {
                                Ok(StdinRead::Data(buffer))
                            } else {
                                // EOF with no data
                                Ok(StdinRead::PipeEmpty)
                            }
                        }
                        Err(err) => Err(FzzError::IoStdinError(err.to_string(), None)),
                    }
                }
                Err(mpsc::RecvTimeoutError::Timeout) => {
                    // No data received within grace period, treat as empty pipe
                    // (thread will be blocked indefinitely; we detach it)
                    stdout::warn("Waiting for stdin...");
                    drop(handle); // detach thread, it will be killed when process exits
                    Ok(StdinRead::PipeEmpty)
                }
                Err(mpsc::RecvTimeoutError::Disconnected) => {
                    // Thread finished before timeout but channel disconnected? Should not happen.
                    let _ = handle.join();
                    Err(FzzError::IoStdinError(
                        "Failed to read stdin".to_string(),
                        None,
                    ))
                }
            }
        }
        Err(TryRecvError::Disconnected) => {
            // Thread panicked before sending
            let _ = handle.join();
            Err(FzzError::IoStdinError(
                "Failed to read stdin".to_string(),
                None,
            ))
        }
    }
}
