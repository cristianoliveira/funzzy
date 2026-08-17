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
use crate::watches::Watches;
use crate::{config, diagnostics, environment, logging, rules, stdout, watcher};
use std::path::PathBuf;

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
        Action::Init { template: _ } if args.migrate => {
            execute(InitCommand::migrate(cli::watch::DEFAULT_FILENAME))
        }
        Action::Init { template } => {
            execute(InitCommand::new(cli::watch::DEFAULT_FILENAME, template))
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

        Action::Watch { target: ref wanted } => {
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
            .with_hooks(load_hooks(&args.config));
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
                    load_hooks(&args.config),
                    control_socket.as_deref().map(std::path::PathBuf::from),
                );
                match tracker.observe(&runtime) {
                    crate::config_revision::RevisionDecision::New(revision) => {
                        watches.with_revision(revision)
                    }
                    crate::config_revision::RevisionDecision::NoOp => watches,
                }
            };
            match wanted {
                Some(target) => match watches.select_target(target) {
                    Some(selected) => execute_watch_command(selected, args, event_stream.clone()),
                    None => stdout::failure(
                        &format!("No target found for '{}'", target),
                        rules::available_targets(&rules),
                    ),
                },
                None => execute_watch_command(watches, args, event_stream.clone()),
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
            .with_debounce(debounce);
            let plan = match watches.run_target_plan(target) {
                Ok(plan) => plan,
                Err(crate::watches::RunTargetError::Missing(_)) => stdout::failure(
                    &format!("No target found for '{}'", target),
                    rules::available_targets(&rules),
                ),
                Err(error) => stdout::failure("Cannot run target", error.to_string()),
            };

            let shutdown = install_shutdown_signal_handler();
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
            .with_hooks(load_hooks(&args.config));
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
        return output;
    }

    if !result.matched.is_empty() {
        output.push_str("  matched:\n");
        for rule in &result.matched {
            output.push_str(&format!("    - {}\n", rule.name));
            output.push_str(&format!("        cwd: {}\n", rule.cwd));
            output.push_str(&format!(
                "        env keys: [{}]\n",
                rule.environment_keys.join(", ")
            ));
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
    // Debounce and concurrency reuse the exact watch-time parsers.
    if let Some(debounce) = config::debounce_from_file(&config_path)
        .unwrap_or_else(|err| stdout::failure("Invalid debounce config", err))
    {
        stdout::info(&format!("debounce: {:?}", debounce));
    }
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
fn load_hooks(config_file: &Option<String>) -> config::RunHooks {
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
        return config::RunHooks::default();
    };
    config::hooks_from_file(&path)
        .unwrap_or_else(|err| stdout::failure("Invalid hooks config", err))
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

/// Graceful fatal config shutdown (contract §5): emit the terminal error,
/// publish the terminal `configInvalid` lifecycle transition so control
/// subscribers observe it, reap owned children/services through process
/// ownership, remove the control socket file(s) explicitly (`process::exit`
/// skips the ControlServer Drop), and exit nonzero. Never
/// SIGKILL/panic/self-SIGTERM.
fn fatal_reload(coordinator: &crate::reload_coordinator::ReloadCoordinator, reason: &str) {
    let current = coordinator.current();
    stdout::error(&format!(
        "Fatal configuration error; terminating watcher.\nWorkspace: {}\nReason: {}",
        current.root().display(),
        reason
    ));
    // TASK-0091 AC8: publish the terminal config diagnostic BEFORE the
    // socket closes — subscribers observe `configInvalid`, then disconnect.
    // Best effort: the process exits right after, so a slow subscriber may
    // miss the notification (bounded, "when possible").
    coordinator
        .lifecycle()
        .invalid(current.revision(), reason.to_owned());
    // TASK-0092 AC9: `process::exit` skips destructors, so the ControlServer
    // Drop never removes the socket file. Remove the active (and any
    // prepared-but-uncommitted) socket file(s) explicitly before exit.
    for path in coordinator.socket_paths_to_cleanup() {
        let _ = std::fs::remove_file(path);
    }
    let (signal, grace) = crate::process_owner::shutdown_policy();
    let _ = crate::process_owner::shutdown_all(signal, grace, false);
    std::process::exit(1);
}

/// Builds a fresh `Watches` from a validated config candidate, bound to the
/// new revision (TASK-0090 commit). The candidate's OWN declared policy
/// (concurrency/debounce/backend/gitignore/hooks) is parsed from the content;
/// missing keys keep the startup defaults — so a policy change committed by
/// the reload is actually applied to post-commit generations (TASK-0092).
fn build_watches_from_content(
    content: &str,
    root: &std::path::Path,
    defaults: &crate::reload::PolicyDefaults,
    revision: crate::config_revision::ConfigRevision,
) -> Result<Watches, String> {
    let rules = crate::config::from_yaml(content).map_err(|err| err.to_string())?;
    crate::rules::validate_rules(&rules).map_err(|err| err.to_string())?;
    let concurrency = crate::config::concurrency_from_yaml(content)
        .map_err(|err| err.to_string())?
        .unwrap_or(defaults.concurrency);
    let debounce = crate::config::debounce_from_yaml(content)
        .map_err(|err| err.to_string())?
        .unwrap_or(defaults.debounce);
    let backend = crate::config::watch_backend_from_yaml(content)
        .map_err(|err| err.to_string())?
        .unwrap_or(defaults.backend.clone());
    let respect_gitignore =
        crate::config::respect_gitignore_from_yaml(content).map_err(|err| err.to_string())?;
    let hooks = crate::config::hooks_from_yaml(content).map_err(|err| err.to_string())?;
    Ok(
        Watches::with_root_and_concurrency(rules, root.to_path_buf(), concurrency)
            .with_debounce(debounce)
            .with_backend(backend)
            .with_gitignore(respect_gitignore)
            .with_hooks(hooks)
            .with_revision(revision),
    )
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
    let baselines: std::collections::HashMap<String, std::time::SystemTime> = config_file_paths
        .iter()
        .filter_map(|path| {
            std::fs::metadata(path)
                .and_then(|metadata| metadata.modified())
                .ok()
                .map(|modified| (path.clone(), modified))
        })
        .collect();
    let startup_config_paths = config_file_paths.clone();
    // AC9: the reload watcher anchors to the config paths' PARENT directories
    // (not the file paths themselves), so atomic editor saves (rename over the
    // destination) and delete/recreate resolve to the canonical path and are
    // still observed after any root swap. Events are then filtered to the
    // exact config filenames below.
    let config_watch_roots: Vec<String> = {
        let mut parents: Vec<PathBuf> = startup_config_paths
            .iter()
            .filter_map(|path| {
                std::path::Path::new(path)
                    .parent()
                    .filter(|parent| !parent.as_os_str().is_empty())
                    .map(|parent| parent.to_path_buf())
            })
            .collect();
        parents.sort();
        parents.dedup();
        parents
            .into_iter()
            .map(|p| p.display().to_string())
            .collect()
    };
    let reload_config_paths = startup_config_paths.clone();
    let reload_coordinator = coordinator.clone();
    let reload_root = watches.root().to_path_buf();
    // TASK-0092: the startup policy doubles as the reload defaults for keys
    // a candidate does not declare. The candidate's OWN declared policy is
    // parsed from its content (see `reload::validate_candidate`), so a
    // concurrency/debounce/backend/gitignore/hooks/socket change is a real
    // semantic change and is applied at commit.
    let reload_defaults = crate::reload::PolicyDefaults {
        concurrency: watches.concurrency(),
        debounce,
        backend: watches.backend(),
        gitignore: watches.respects_gitignore(),
        hooks: watches.hooks(),
    };
    // AC8: the current control socket path (as configured at startup); the
    // reload thread detects candidate path changes and requests a
    // bind-new-before-retire-old handoff through the coordinator.
    let reload_current_socket = args.control_socket.clone();
    let initial_revision = watches.revision().cloned();
    // TASK-0090: the reload watcher signals readiness after registering its
    // config-path roots; the main loop gates init on it so a config-touching
    // init task never fires before the reload watcher is subscribed.
    let (reload_ready_tx, reload_ready_rx) = std::sync::mpsc::channel();
    let th = std::thread::spawn(move || {
        let baselines = std::sync::Mutex::new(baselines);
        let backend = crate::watcher::WatchBackend::Auto;
        let tracker = std::sync::Mutex::new(crate::config_revision::RevisionTracker::new());
        // Seed the reload tracker with the initial revision the composition
        // root already observed, so reload revision numbers continue
        // monotonically from startup (never two trackers disagreeing).
        if let Some(initial) = initial_revision {
            tracker.lock().unwrap().seed(initial);
        }
        let reload_ready_tx = reload_ready_tx;
        let reload_config_paths = reload_config_paths;
        let reload_current_socket = reload_current_socket;
        watcher::events(
            config_watch_roots,
            move || {
                let _ = reload_ready_tx.send(());
            },
            move |_batch_id: u64, events: &[watcher::FileEvent]| {
                // AC9: only events targeting the canonical config paths (or
                // their parents' watched subtrees) trigger validation. Atomic
                // editor saves surface as a change on the config filename
                // under the watched parent; unrelated files never validate.
                let file_changed = events
                    .iter()
                    .map(|event| event.path.clone())
                    .find(|path| {
                        reload_config_paths.iter().any(|candidate| {
                            path == candidate
                                || path.ends_with(candidate)
                                || std::path::Path::new(path)
                                    .canonicalize()
                                    .ok()
                                    .map(|p| p.to_string_lossy().to_string())
                                    .as_deref()
                                    == Some(candidate.as_str())
                        })
                    })
                    .unwrap_or_default();
                if file_changed.is_empty() {
                    return;
                }

                // Ignore events that do not reflect a real modification since
                // the watcher started (historical FSEvents replays).
                let mut baselines = baselines.lock().unwrap();
                let current = std::fs::metadata(&file_changed)
                    .and_then(|metadata| metadata.modified())
                    .ok();
                let baseline = baselines.get(&file_changed).copied();
                let changed = match (current, baseline) {
                    (Some(current), Some(baseline)) => current != baseline,
                    // Unknown path or missing metadata: treat as real.
                    _ => true,
                };
                if !changed {
                    return;
                }
                if let Some(current) = current {
                    baselines.insert(file_changed.clone(), current);
                }
                drop(baselines);

                // Contract §2: read the candidate only after the window
                // settles; a partial write fails validation instead of being
                // misclassified.
                let content = match std::fs::read_to_string(&file_changed) {
                    Ok(content) => content,
                    Err(err) => {
                        // Config deleted/renamed: treat as invalid (contract
                        // §7) — the watcher cannot run without a config.
                        fatal_reload(
                            &reload_coordinator,
                            &format!("config unreadable after change: {err}"),
                        );
                        return;
                    }
                };

                // AC8: parse the candidate's control socket path up front so
                // it participates in the semantic decision (a socket move is
                // a real revision change, never a no-op).
                let candidate_socket = config::control_socket_from_yaml(&content)
                    .unwrap_or_else(|err| {
                        stdout::warn(&format!("Cannot read socket from candidate: {err}"));
                        None
                    })
                    .map(std::path::PathBuf::from);

                match crate::reload::decide(
                    &mut tracker.lock().unwrap(),
                    &content,
                    reload_root.clone(),
                    &reload_defaults,
                ) {
                    crate::reload::ReloadDecision::NoOp => {
                        stdout::info("Config save has no semantic change; nothing to reload.");
                    }
                    crate::reload::ReloadDecision::Commit(revision) => {
                        // TASK-0091 AC3: the reload lifecycle transitions
                        // only when a candidate actually commits (never for a
                        // no-op save): `configReloading` before prepare,
                        // `configReloaded` after the commit boundary.
                        reload_coordinator.lifecycle().reloading(Some(&revision));
                        let candidate_watches = build_watches_from_content(
                            &content,
                            &reload_root,
                            &reload_defaults,
                            revision.clone(),
                        );
                        match candidate_watches {
                            Ok(candidate) => {
                                let log_sink = |msg: &str| stdout::warn(msg);
                                // AC8: if the candidate changes the control
                                // socket path, bind the NEW socket before
                                // commit (failure is fatal — never a silent
                                // stale socket) and retire the OLD one after.
                                let socket_changed = match (
                                    reload_current_socket.as_deref(),
                                    candidate_socket.as_deref(),
                                ) {
                                    (Some(current), Some(candidate)) => current != candidate,
                                    (None, Some(_)) | (Some(_), None) => true,
                                    (None, None) => false,
                                };
                                if socket_changed {
                                    if let Some(new_path) = candidate_socket.as_deref() {
                                        if let Err(err) =
                                            reload_coordinator.prepare_socket(new_path)
                                        {
                                            fatal_reload(
                                                &reload_coordinator,
                                                &format!("control socket rebind failed: {err}"),
                                            );
                                            return;
                                        }
                                    }
                                }
                                // Prepare→commit→retire (contract §4): added
                                // roots register on the live backend BEFORE
                                // the pointer swap; any prepare failure takes
                                // the invalid fatal path with nothing mutated.
                                let transaction = match reload_coordinator.begin(
                                    revision.clone(),
                                    candidate,
                                    &log_sink,
                                    candidate_socket.clone(),
                                ) {
                                    Ok(transaction) => transaction,
                                    Err(err) => {
                                        fatal_reload(
                                            &reload_coordinator,
                                            &format!("reload prepare failed: {err}"),
                                        );
                                        return;
                                    }
                                };
                                if let Err(err) = reload_coordinator.commit(&transaction) {
                                    fatal_reload(
                                        &reload_coordinator,
                                        &format!("reload commit failed: {err}"),
                                    );
                                    return;
                                }
                                // AC10: truncate-on-change fires only after a
                                // committed valid semantic reload, preserving
                                // the deterministic notice order (truncate
                                // notice precedes the reload notice).
                                if truncate_on_config_change {
                                    match logging::truncate() {
                                        Ok(()) => stdout::info(
                                            "Log file truncated before reloading configuration.",
                                        ),
                                        Err(err) => stdout::warn(&format!(
                                            "Failed to truncate log file: {err}"
                                        )),
                                    }
                                }
                                // Obsolete roots/backend resources retire only
                                // after the commit boundary (contract §4).
                                if let Err(err) = reload_coordinator.retire(&transaction, &log_sink)
                                {
                                    stdout::warn(&format!("reload retire warning: {err}"));
                                }
                                // AC8: retire the OLD control socket after the
                                // boundary; its file is removed by the server
                                // drop, and the new socket is already live.
                                reload_coordinator.retire_socket();
                                // The commit (shared config swap + worker
                                // revision + backend root swap) completed;
                                // only now is the reload observable (contract
                                // §4 live point = atomic commit).
                                stdout::info(&format!(
                                    "Config change is valid; hot-reloading to revision {}.",
                                    revision.number
                                ));
                                // The commit boundary passed; the new revision
                                // is live and observable.
                                reload_coordinator.lifecycle().reloaded(&revision);
                            }
                            Err(err) => fatal_reload(&reload_coordinator, &err),
                        }
                    }
                    crate::reload::ReloadDecision::Fatal(error) => {
                        fatal_reload(
                            &reload_coordinator,
                            &format!(
                                "invalid config ({}): {}",
                                match error.gate {
                                    crate::reload::ValidationGate::Syntactic => "syntax",
                                    crate::reload::ValidationGate::Schema => "schema",
                                    crate::reload::ValidationGate::Semantic => "semantics",
                                    crate::reload::ValidationGate::Operational => "operational",
                                },
                                error.reason
                            ),
                        );
                    }
                }
            },
            debounce,
            backend,
            false,
            None,
        )
    });

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
        );
    }
    if non_block {
        // Task children lead their own process groups (cmd::spawn_configured),
        // so SIGINT/SIGTERM to funzzy's foreground group no longer reaches
        // them. Catch both and route through the shared ownership path before
        // exit so no descendant is orphaned (TASK-0030).
        let _shutdown = install_shutdown_signal_handler();
        execute(
            WatchNonBlockCommand::with_events(
                watches,
                verbose,
                fail_fast,
                run_on_init,
                args.control_socket.map(std::path::PathBuf::from),
                event_stream,
            )
            .with_reload(coordinator.clone())
            .with_reload_ready(reload_ready_rx),
        )
    } else {
        execute(WatchCommand::with_events(
            watches,
            verbose,
            fail_fast,
            run_on_init,
            event_stream,
        ))
    }

    let _ = th.join().expect("Failed to join config watcher thread");
}

/// One deterministic startup record (TASK-0023): config path, workspace
/// root, watch roots, task count, busy policy, run-on-init state, log
/// destination, and control socket. The summary is compact and never dumps
/// the full configuration.
fn emit_startup_record(
    watches: &Watches,
    config_file_paths: &[String],
    control_socket: Option<&str>,
    log_file: Option<&str>,
    run_on_init: bool,
    non_block: bool,
) {
    let config_path = config_file_paths
        .first()
        .cloned()
        .unwrap_or_else(|| "default".to_owned());
    let watch_roots = watches.paths_to_watch().unwrap_or_default().join(",");
    let policy = if non_block { "restart" } else { "wait" };
    diagnostics::debug(&diagnostics::Record {
        source: Some("config"),
        decision: Some("startup"),
        path: Some(config_path),
        note: Some(format!(
            "workspace={} tasks={} policy={} run_on_init={} log={} socket={} watch_roots={}",
            watches.root().display(),
            watches.targets().len(),
            policy,
            run_on_init,
            log_file.unwrap_or("none"),
            control_socket.unwrap_or("none"),
            watch_roots,
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
fn install_shutdown_signal_handler() -> std::sync::Arc<std::sync::atomic::AtomicI32> {
    use nix::sys::signal::{sigaction, SaFlags, SigAction, SigHandler, SigSet};
    use nix::unistd;
    use std::sync::atomic::{AtomicI32, Ordering};
    use std::sync::Arc;

    /// Self-pipe write end handed to the signal handler (TASK-0030). The
    /// handler performs only the async-signal-safe write(2); all real work
    /// happens on a normal thread after the byte arrives.
    static WAKE_WRITE_FD: std::sync::OnceLock<i32> = std::sync::OnceLock::new();

    extern "C" fn on_shutdown_signal(signal: nix::libc::c_int) {
        // Async-signal-safe: one best-effort byte; nothing else.
        if let Some(fd) = WAKE_WRITE_FD.get() {
            let _ = unistd::write(*fd, &[signal as u8]);
        }
    }

    let exit_code = Arc::new(AtomicI32::new(0));
    let signal_exit_code = Arc::clone(&exit_code);

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
    let _ = fcntl(wake_read_fd, F_SETFD(FdFlag::FD_CLOEXEC));
    let _ = fcntl(wake_write_fd, F_SETFD(FdFlag::FD_CLOEXEC));
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
            match unistd::read(wake_read_fd, &mut signal_byte) {
                Ok(_) => {
                    let code = if signal_byte[0] == Signal::SIGINT as u8 {
                        130
                    } else {
                        143
                    };
                    signal_exit_code.store(code, Ordering::SeqCst);
                    let (signal, grace) = crate::process_owner::shutdown_policy();
                    let _ = crate::process_owner::shutdown_all(signal, grace, false);
                    std::process::exit(code);
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
