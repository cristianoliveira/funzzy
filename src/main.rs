// #![feature(plugin)]
// #![plugin(clippy)]
// #![deny(clippy)]
extern crate docopt;
extern crate serde_derive;

mod cli;
mod cmd;
mod errors;
mod rules;
mod stdout;
mod watcher;
mod watches;
mod workers;
mod yaml;

use cli::*;
use errors::FzzError;
use nix::{
    sys::signal::{self, Signal},
    unistd::Pid,
};
use watches::Watches;

use serde_derive::Deserialize;
use std::io;
#[warn(unused_imports)]
use std::io::prelude::*;

use docopt::Docopt;

const SHA: Option<&str> = option_env!("GITSHA");
const VERSION: &str = env!("CARGO_PKG_VERSION");
const USAGE: &str = "
Funzzy the watcher.

Alias:
  fzz -> funzzy

Usage:
  funzzy [options]
  funzzy init
  funzzy watch [<command>] [options]
  funzzy <command> [options]

Commands:
    init                Create a new '.watch.yaml' file.
    watch               Watch for file changes and execute a command.

Options:
  <command>               Run an arbitrary command for current folder.
  -c --config=<cfgfile>   Use given config file.
  -t --target=<task>      Execute only the given task target.
  -n --non-block          Execute tasks and cancel them if a new event is received.
  -b --fail-fast          Bail current execution if a task fails (exit code != 0).
  -h --help               Show this message.
  -v --version            Show version.
  -V                      Use verbose output.
";

#[allow(non_snake_case)]
#[derive(Debug, Deserialize)]
pub struct Args {
    // comand
    pub cmd_init: bool,
    pub cmd_watch: bool,

    pub arg_command: String,

    // options
    pub flag_config: String,
    pub flag_target: String,

    pub flag_n: bool,
    pub flag_h: bool,
    pub flag_v: bool,
    pub flag_V: bool,

    pub flag_non_block: bool,
    pub flag_fail_fast: bool,
}

fn main() {
    let args: Args = Docopt::new(USAGE)
        .and_then(|d| d.deserialize())
        .unwrap_or_else(|e| e.exit());

    match args {
        Args { flag_v: true, .. } => show(get_version().as_str()),
        Args { flag_h: true, .. } => show(USAGE),

        // Commands
        Args { cmd_init: true, .. } => execute(InitCommand::new(cli::watch::DEFAULT_FILENAME)),

        Args {
            ref arg_command, ..
        } if !arg_command.is_empty() => {
            match from_stdin() {
                Ok(content) => {
                    if content.trim().is_empty() {
                        show("The list of files received is empty");
                    }

                    let watch_rules = match rules::from_string(content, arg_command.to_string()) {
                        Ok(rules) => rules,
                        Err(err) => error("Failed to get rules from stdin", err),
                    };

                    if let Err(err) = rules::validate_rules(&watch_rules) {
                        error("Invalid config file.", err);
                    }

                    execute_watch_command(Watches::new(watch_rules), args);
                }
                Err(err) => error("Failed to read stdin", err.to_string()),
            };
        }

        _ => {
            let rules = if args.flag_config.is_empty() {
                rules::from_default_file_config().unwrap_or_else(|err| {
                    stdout::error(
                        &vec![
                            "Missing config file",
                            "Expected file .watch.yaml or .watch.yml to be present",
                            "in the current directory",
                            "Debugging:",
                            " - Did you forget to run 'fzz init'?",
                        ]
                        .join("\n"),
                    );

                    error("Failed to read default config file", err);
                })
            } else {
                match rules::from_file(&args.flag_config) {
                    Ok(rules) => rules,
                    Err(err) => error("Failed to read config file", err.to_string()),
                }
            };

            if let Err(err) = rules::validate_rules(&rules) {
                error("Invalid config file.", err);
            }

            if !args.flag_target.is_empty() {
                let filtered = rules
                    .iter()
                    .cloned()
                    .filter(|r| r.name.contains(&args.flag_target))
                    .collect::<Vec<rules::Rules>>();

                if filtered.is_empty() {
                    let mut output = String::new();
                    output.push_str(&format!("No target found for '{}'\n\n", args.flag_target));
                    output.push_str("Available targets:\n");
                    output.push_str(&format!(
                        "  {}\n",
                        rules
                            .iter()
                            .cloned()
                            .map(|r| r.name)
                            .collect::<Vec<String>>()
                            .join("\n  ")
                    ));

                    stdout::info(output.as_str());

                    show("Finished there is no task to run.");
                } else {
                    execute_watch_command(Watches::new(filtered), args);
                }
            } else {
                execute_watch_command(Watches::new(rules), args);
            }
        }
    };
}

pub fn execute_watch_command(watches: Watches, args: Args) {
    let config_file_paths = if args.flag_config.is_empty() {
        let dir = std::env::current_dir().expect("Failed to get current directory");
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
    } else {
        vec![format!("{}", &args.flag_config)]
    };

    // This here restarts the watcher if the config file changes
    let watcher_pid = std::process::id();
    let th = std::thread::spawn(move || {
        watcher::events(
            config_file_paths,
            |file_changed| {
                stdout::warn(
                    &vec![
                        "The config file has changed while an instance was running.",
                        &format!("Config file: {}", file_changed),
                    ]
                    .join("\n"),
                );

                println!("Watcher PID: {}", watcher_pid);

                match signal::kill(Pid::from_raw(watcher_pid as i32), Signal::SIGTERM) {
                    Ok(_) => stdout::info("Terminating funzzy..."),
                    Err(err) => panic!("Failed to terminate watcher forcefully.\nCause: {:?}", err),
                }
            },
            false,
        )
    });

    let verbose = args.flag_V;
    let fail_fast = args.flag_fail_fast;
    if args.flag_non_block {
        execute(WatchNonBlockCommand::new(watches, verbose, fail_fast))
    } else {
        execute(WatchCommand::new(watches, verbose, fail_fast))
    }

    let _ = th.join().expect("Failed to join config watcher thread");
}

fn execute<T: Command>(command: T) {
    if let Err(err) = command.execute() {
        error("Command failed to execute", err.to_string());
    }
}

fn from_stdin() -> errors::Result<String> {
    let stdin = io::stdin();

    // Spawn a thread and if there is no input in 5 seconds kills the process
    let has_input = std::sync::Arc::new(std::sync::Mutex::new(false));
    let clone_has_input = has_input.clone();
    std::thread::spawn(move || {
        std::thread::sleep(std::time::Duration::from_secs(1));

        let had_input = *clone_has_input.lock().expect("Could not lock has_input");
        if !had_input {
            error(
                "Timed out waiting for input.",
                vec![
                    "Hint: Did you forget to pipe an output of a command?".to_string(),
                    "Try `find . | fzz 'echo \"changed: {{filepath}}\"'`".to_string(),
                ]
                .join("\n"),
            );
        }
    });

    let mut buffer = String::new();
    match stdin.lock().read_to_string(&mut buffer) {
        Ok(bytes) => {
            let mut has_input_mutex = match has_input.lock() {
                Ok(mutex) => mutex,
                Err(err) => {
                    return Err(FzzError::IoStdinError(err.to_string(), None));
                }
            };

            *has_input_mutex = bytes > 0;
            if bytes > 0 {
                Ok(buffer)
            } else {
                Err(FzzError::IoStdinError(
                    "Timed out waiting for input.".to_string(),
                    Some(
                        vec![
                            "Did you forget to pipe an output of a command?".to_string(),
                            "Try `find . | fzz 'echo \"changed: {{filepath}}\"'`".to_string(),
                        ]
                        .join(" "),
                    ),
                ))
            }
        }
        Err(err) => Err(FzzError::IoStdinError(err.to_string(), None)),
    }
}

fn get_version() -> String {
    if let Some(sha) = SHA {
        format!("{}-{}", VERSION, sha)
    } else {
        VERSION.to_owned()
    }
}

fn show(text: &str) -> ! {
    println!("{}", text);
    std::process::exit(0);
}

fn error(text: &str, err: String) -> ! {
    println!("Error: {}", text);
    println!("{}", err);
    std::process::exit(1);
}
