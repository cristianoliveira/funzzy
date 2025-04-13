// #![feature(plugin)]
// #![plugin(clippy)]
// #![deny(clippy)]
extern crate docopt;
extern crate serde_derive;

mod cli;
mod cmd;
mod environment;
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
use docopt::Error;

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
  -c --config <cfgfile>   Use given config file.
  -t --target <name>      Execute only the given task target (if empty list availables).
  -n --non-block          Execute tasks and cancel them if a new event is received.
  -b --fail-fast          Bail current execution if a task fails (exit code != 0).
  --no-run-on-init        Do not run tasks on initialization.
  -h --help               Show this message.
  -v --version            Show version.
  -V                      Use verbose output.

Environment configs:

FUNZZY_NON_BLOCK: Boolean   Same as `--non-block`
FUNZZY_BAIL: Boolean        Same as `--fail-fast`
FUNZZY_COLORED: Boolean     Output with colors.
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
    pub flag_target: Option<String>,

    pub flag_n: bool,
    pub flag_h: bool,
    pub flag_v: bool,
    pub flag_V: bool,

    pub flag_non_block: bool,
    pub flag_no_run_on_init: bool,
    pub flag_no_run_on_init: bool,
    pub flag_fail_fast: bool,
}

fn main() {
    let docopt = Docopt::new(USAGE);
    let docopt_parser = match docopt {
        Ok(args) => args,
        Err(err) => {
            panic!("Failed to parse arguments: {:?}", err);
        }
    };

    let args: Args = match docopt_parser.deserialize() {
        Ok(args) => args,
        Err(err) => {
            match err {
                Error::WithProgramUsage(err, usage) => {
                    match *err {
                        Error::Help => stdout::show_and_exit(&usage),
                        Error::Version(_) => stdout::show_and_exit(get_version().as_str()),
                        Error::Argv(argverr) => {
                            // NOTE: Docopt doesn't support a flag without a value even when
                            // declaring a default value with [default: foobar].
                            // In short, this adds a default value to the flag `--target` if it's empty.
                            // So one can use `fzz -t` and it will list all available tasks.
                            if !argverr.contains("flag '--target' but reached end of arguments") {
                                stdout::failure(&argverr, usage);
                            }

                            let argv_with_missing_target = std::env::args()
                                .flat_map(|arg| {
                                    if arg == "--target" || arg == "-t" {
                                        vec![arg, "".to_string()]
                                    } else {
                                        vec![arg]
                                    }
                                })
                                .collect::<Vec<String>>();

                            let newargs: Args = Docopt::new(USAGE)
                                .and_then(|d| d.argv(argv_with_missing_target).deserialize())
                                .unwrap_or_else(|err| {
                                    stdout::failure("Failed to parse arguments", err.to_string())
                                });

                            newargs
                        }
                        _ => stdout::failure(&usage, err.to_string()),
                    }
                }
                err => stdout::failure("Failed to parse arguments", err.to_string()),
            }
        }
    };

    match args {
        Args { flag_v: true, .. } => stdout::show_and_exit(get_version().as_str()),
        // Commands
        Args { cmd_init: true, .. } => execute(InitCommand::new(cli::watch::DEFAULT_FILENAME)),

        Args {
            ref arg_command, ..
        } if !arg_command.is_empty() => {
            match from_stdin() {
                Ok(content) => {
                    if content.trim().is_empty() {
                        stdout::show_and_exit("The list of files received is empty");
                    }

                    let patterns = match rules::extract_paths(content) {
                        Ok(patterns) => patterns,
                        Err(err) => {
                            stdout::failure("Failed to get rules from stdin", err.to_string())
                        }
                    };

                    let watch_rules = match rules::from_string(patterns, arg_command.to_string()) {
                        Ok(rules) => rules,
                        Err(err) => {
                            stdout::failure("Failed to get rules from stdin", err.to_string())
                        }
                    };

                    if let Err(err) = rules::validate_rules(&watch_rules) {
                        stdout::failure("Invalid config file.", err);
                    }

                    execute_watch_command(Watches::new(watch_rules), args);
                }
                Err(err) => stdout::failure("Failed to read stdin", err.to_string()),
            };
        }

        _ => {
            let rules = if args.flag_config.is_empty() {
                rules::from_default_file_config().unwrap_or_else(|err| {
                    stdout::failure("Failed to read default config file", err.to_string());
                })
            } else {
                match rules::from_file(&args.flag_config) {
                    Ok(rules) => rules,
                    Err(err) => stdout::failure("Failed to read config file", err.to_string()),
                }
            };

            if let Err(err) = rules::validate_rules(&rules) {
                stdout::failure("Invalid config file.", err);
            }

            match args.flag_target {
                Some(ref target) if target.trim().is_empty() => {
                    stdout::info(&format!(
                        "`--target` help\n{}",
                        rules::available_targets(rules)
                    ));
                    stdout::show_and_exit("Usage `fzz -t <text_contain_in_task>`");
                }
                Some(ref target) => {
                    let filtered = rules
                        .iter()
                        .cloned()
                        .filter(|r| r.name.contains(target))
                        .collect::<Vec<rules::Rules>>();

                    if filtered.is_empty() {
                        stdout::failure(
                            &format!("No target found for '{}'", target),
                            rules::available_targets(rules),
                        );
                    } else {
                        execute_watch_command(Watches::new(filtered), args);
                    }
                }
                _ => execute_watch_command(Watches::new(rules), args),
            }
        }
    };
}

pub fn execute_watch_command(watches: Watches, args: Args) {
    let possible_config_paths = if args.flag_config.is_empty() {
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

    let config_file_paths = possible_config_paths
        .into_iter()
        .filter(|path| std::path::Path::new(path).exists())
        .collect::<Vec<String>>();

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
    let fail_fast = args.flag_fail_fast || environment::is_enabled("FUNZZY_BAIL");
    let fail_fast_env = args.flag_non_block || environment::is_enabled("FUNZZY_NON_BLOCK");
    let run_on_init = !args.flag_no_run_on_init;
    if fail_fast_env {
        execute(WatchNonBlockCommand::new(watches, verbose, fail_fast, run_on_init))
    } else {
        execute(WatchCommand::new(watches, verbose, fail_fast, run_on_init))
    }

    let _ = th.join().expect("Failed to join config watcher thread");
}

fn execute<T: Command>(command: T) {
    if let Err(err) = command.execute() {
        stdout::failure("Command failed to execute", err.to_string());
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
            stdout::failure(
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
