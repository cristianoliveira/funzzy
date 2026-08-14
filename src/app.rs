//! Application entry point: CLI parsing, command dispatch, and watch startup.
//!
//! This module is the canonical home of executable behavior. `src/main.rs`
//! is a thin process adapter that calls [`run`]; integration tests exercise
//! the same modules the binaries use.

use crate::arguments::{Action, Arguments, OnBusy};
use crate::cli;
use crate::cli::*;
use crate::errors;
use crate::errors::FzzError;
use crate::watches::Watches;
use crate::{config, environment, logging, rules, stdout, watcher};

use nix::{
    sys::signal::{self, Signal},
    unistd::Pid,
};
use std::io::prelude::*;
use std::io::{self, IsTerminal};
use std::process;
use std::sync::mpsc;
use std::sync::mpsc::TryRecvError;

/// Runs the application: parse arguments, choose the execution path, and
/// start the watcher or exit with a message.
pub fn run() {
    let args = Arguments::parse();

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

    // Resolve the workspace root once and anchor all watch planning,
    // config discovery, and command template preparation to it.
    let workspace_root = std::env::current_dir().expect("Failed to get current directory");

    match args.action {
        // Commands
        Action::Init if args.migrate => execute(InitCommand::migrate(cli::watch::DEFAULT_FILENAME)),
        Action::Init => execute(InitCommand::new(cli::watch::DEFAULT_FILENAME)),

        Action::Watch { target: ref wanted } => {
            let rules = load_rules(&args.config);
            if let Err(err) = rules::validate_rules(&rules) {
                stdout::failure("Invalid config file.", err);
            }
            match wanted {
                Some(target) => {
                    let filtered = rules
                        .iter()
                        .cloned()
                        .filter(|r| r.name.contains(target))
                        .collect::<Vec<rules::Rules>>();
                    if filtered.is_empty() {
                        stdout::failure(
                            &format!("No target found for '{}'", target),
                            rules::available_targets(&rules),
                        );
                    } else {
                        execute_watch_command(
                            Watches::with_root(filtered, workspace_root.clone()),
                            args,
                        );
                    }
                }
                None => {
                    execute_watch_command(Watches::with_root(rules, workspace_root.clone()), args)
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
        Action::Explain { ref path } => {
            let rules = load_rules(&args.config);
            if let Err(err) = rules::validate_rules(&rules) {
                stdout::failure("Invalid config file.", err);
            }
            let watches = Watches::with_root(rules, workspace_root);
            let result = watches.explain(path);
            stdout::info(&explain_output(path, &result));
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

                    execute_watch_command(Watches::with_root(watch_rules, workspace_root), args);
                }
                Err(err) => stdout::failure("Failed to read stdin", err.to_string()),
            };
        }
    }
}

/// Renders a deterministic human summary of which tasks a path matches.
/// Reuses the structured `ExplainResult`; no matching logic lives here.
fn explain_output(path: &str, result: &crate::watches::ExplainResult) -> String {
    let mut output = String::from("Explain path ");
    output.push_str(path);
    output.push('\n');

    if result.matched.is_empty() && result.ignored.is_empty() {
        output.push_str("  unmatched: no configured task watches this path\n");
        return output;
    }

    if !result.matched.is_empty() {
        output.push_str("  matched:\n");
        for rule in &result.matched {
            output.push_str(&format!("    - {}\n", rule.name));
            for change in &rule.change_patterns {
                output.push_str(&format!("        change: {}\n", change));
            }
        }
    }

    if !result.ignored.is_empty() {
        output.push_str("  ignored:\n");
        for rule in &result.ignored {
            output.push_str(&format!("    - {}\n", rule.name));
            for change in &rule.change_patterns {
                output.push_str(&format!("        change: {}\n", change));
            }
            for ignore in &rule.ignore_patterns {
                output.push_str(&format!("        ignored by: {}\n", ignore));
            }
        }
    }

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

fn execute_watch_command(watches: Watches, mut args: Arguments) {
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

    let config_file_paths = possible_config_paths
        .into_iter()
        .filter(|path| std::path::Path::new(path).exists())
        .collect::<Vec<String>>();

    if args.control_socket.is_none() {
        args.control_socket = config_file_paths
            .first()
            .map(|path| config::control_socket_from_file(path))
            .transpose()
            .unwrap_or_else(|err| stdout::failure("Invalid control socket config", err))
            .flatten();
    }

    // This here restarts the watcher if the config file changes
    let watcher_pid = std::process::id();
    let th = std::thread::spawn(move || {
        watcher::events(
            config_file_paths,
            || {},
            move |file_changed| {
                let truncation_status = if truncate_on_config_change {
                    Some(logging::truncate())
                } else {
                    None
                };

                stdout::warn(
                    &vec![
                        "The config file has changed while an instance was running.",
                        &format!("Config file: {}", file_changed),
                    ]
                    .join("\n"),
                );

                if let Some(Ok(())) = truncation_status {
                    stdout::info("Log file truncated before reloading configuration.");
                } else if let Some(Err(err)) = truncation_status {
                    stdout::warn(&format!("Failed to truncate log file: {}", err));
                }

                println!("Watcher PID: {}", watcher_pid);
                logging::log_line(&format!("Watcher PID: {}", watcher_pid));

                match signal::kill(Pid::from_raw(watcher_pid as i32), Signal::SIGTERM) {
                    Ok(_) => stdout::info("Terminating funzzy..."),
                    Err(err) => panic!("Failed to terminate watcher forcefully.\nCause: {:?}", err),
                }
            },
            false,
        )
    });

    let verbose = args.verbose;
    let fail_fast = args.fail_fast || environment::is_enabled("FUNZZY_BAIL");
    let non_block = matches!(args.on_busy, OnBusy::Restart)
        || environment::is_enabled("FUNZZY_NON_BLOCK")
        || args.control_socket.is_some();

    let run_on_init = !args.no_run_on_init;
    if non_block {
        // Task children lead their own process groups (cmd::spawn_configured),
        // so SIGINT/SIGTERM to funzzy's foreground group no longer reaches
        // them. Catch both and route through the shared ownership path before
        // exit so no descendant is orphaned (TASK-0030).
        install_shutdown_signal_handler();
        execute(WatchNonBlockCommand::new(
            watches,
            verbose,
            fail_fast,
            run_on_init,
            args.control_socket.map(std::path::PathBuf::from),
        ))
    } else {
        execute(WatchCommand::new(watches, verbose, fail_fast, run_on_init))
    }

    let _ = th.join().expect("Failed to join config watcher thread");
}

/// Catches SIGINT and SIGTERM on a dedicated thread and routes them through
/// the shared process-group ownership path (`process_owner::shutdown_all`)
/// before exiting, so descendants in their own groups are not orphaned.
///
/// Blocking is installed only on the non-block path: blocking-mode children
/// share funzzy's process group, so the terminal's Ctrl-C still reaches them
/// naturally.
fn install_shutdown_signal_handler() {
    use nix::sys::signal::{sigprocmask, SigSet, SigmaskHow};

    let mut mask = SigSet::empty();
    mask.add(Signal::SIGINT);
    mask.add(Signal::SIGTERM);
    // Block process-wide so the default action (terminate) does not fire; the
    // sigwait thread below drains pending signals.
    let _ = sigprocmask(SigmaskHow::SIG_BLOCK, Some(&mask), None);

    std::thread::spawn(move || {
        let mut set = SigSet::empty();
        set.add(Signal::SIGINT);
        set.add(Signal::SIGTERM);
        loop {
            match set.wait() {
                Ok(Signal::SIGINT) => {
                    let (signal, grace) = crate::process_owner::shutdown_policy();
                    let _ = crate::process_owner::shutdown_all(signal, grace, false);
                    std::process::exit(130);
                }
                Ok(Signal::SIGTERM) => {
                    let (signal, grace) = crate::process_owner::shutdown_policy();
                    let _ = crate::process_owner::shutdown_all(signal, grace, false);
                    std::process::exit(143);
                }
                Ok(_) => continue,
                Err(_) => continue,
            }
        }
    });
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
