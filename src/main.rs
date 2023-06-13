// #![feature(plugin)]
// #![plugin(clippy)]
// #![deny(clippy)]
extern crate docopt;
extern crate serde_derive;

mod cli;
mod cmd;
mod rules;
mod stdout;
mod watcher;
mod watches;
mod workers;
mod yaml;

use cli::*;
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

Usage:
  funzzy [options]
  funzzy init
  funzzy watch [<command>] [options]
  funzzy run <command> <interval> (*deprecated*)
  funzzy <command> [options]

Commands:
    init                Create a new funzzy.yml file.
    watch               Watch for file changes and execute a command.

Options:
  <command>            Run an arbitrary command for current folder.
  --config=<cfgfile>   Use given config file.
  --target=<task>      Execute only the given task target.
  -n --non-block       Execute tasks and cancel them if a new event is received.
  -h --help            Shows this message.
  -v --version         Shows version.
  -V                   Use verbose output.
  -c                   Execute given command for current folder.
";

#[allow(non_snake_case)]
#[derive(Debug, Deserialize)]
pub struct Args {
    // comand
    pub cmd_init: bool,
    pub cmd_watch: bool,

    pub arg_command: String,
    pub arg_interval: u64,

    // options
    pub flag_config: String,
    pub flag_target: String,

    pub flag_n: bool,
    pub flag_c: bool,
    pub flag_h: bool,
    pub flag_v: bool,
    pub flag_V: bool,
}

fn main() {
    let args: Args = Docopt::new(USAGE)
        .and_then(|d| d.deserialize())
        .unwrap_or_else(|e| e.exit());

    match args {
        // Metainfo
        Args { flag_v: true, .. } => show(get_version().as_str()),
        Args { flag_h: true, .. } => show(USAGE),

        // Commands
        Args { cmd_init: true, .. } => execute(InitCommand::new(cli::watch::DEFAULT_FILENAME)),

        Args { arg_command, .. } if !arg_command.is_empty() => {
            match from_stdin() {
                Ok(content) => {
                    if content.trim().is_empty() {
                        show("The list of files received is empty");
                    }

                    execute_watch_command(
                        Watches::new(rules::from_string(content, arg_command)),
                        args.flag_n,
                        args.flag_V,
                    );
                }
                Err(err) => error("Error while greading stdin", err),
            };
        }

        _ => {
            let filename = if args.flag_config.is_empty() {
                cli::watch::DEFAULT_FILENAME
            } else {
                &args.flag_config
            };

            match rules::from_file(filename) {
                Ok(rules) => {
                    if !args.flag_target.is_empty() {
                        let filtered = rules
                            .iter()
                            .cloned()
                            .filter(|r| r.name.contains(&args.flag_target))
                            .collect::<Vec<rules::Rules>>();

                        if filtered.is_empty() {
                            let mut output = String::new();
                            output.push_str(&format!("No target found for {}", args.flag_target));
                            output.push_str(&"Available targets:");

                            for rule in rules {
                                output.push_str(&format!("  {}", rule.name));
                            }

                            output.push_str(&"Finished there is no task to run");
                            show(output.as_str());
                        } else {
                            execute_watch_command(Watches::new(filtered), args.flag_n, args.flag_V);
                        }
                    } else {
                        execute_watch_command(Watches::new(rules), args.flag_n, args.flag_V);
                    }
                }

                Err(err) => error("Error while reading config file", err),
            }
        }
    }
}

pub fn execute_watch_command(watches: Watches, non_block: bool, verbose: bool) {
    if non_block {
        execute(WatchNonBlockCommand::new(watches, verbose))
    } else {
        execute(WatchCommand::new(watches, verbose))
    }
}

fn execute<T: Command>(command: T) {
    if let Err(err) = command.execute() {
        stdout::error(err.as_str());
    }
}

fn from_stdin() -> Result<String, String> {
    let stdin = io::stdin();

    // Spawn a thread and if there is no input in 5 seconds kills the process
    let has_input = std::sync::Arc::new(std::sync::Mutex::new(false));
    let clone_has_input = has_input.clone();
    std::thread::spawn(move || {
        std::thread::sleep(std::time::Duration::from_secs(5));

        let had_input = *clone_has_input.lock().expect("Could not lock has_input");
        if !had_input {
            stdout::info("Timed out waiting for input after 5 seconds");
            std::process::exit(0);
        }
    });

    let mut buffer = String::new();
    match stdin.lock().read_to_string(&mut buffer) {
        Ok(bytes) => {
            let mut has_input_mutex = has_input.lock().unwrap();
            *has_input_mutex = bytes > 0;
            if bytes > 0 {
                Ok(buffer)
            } else {
                Err(String::from("There was no inputs"))
            }
        }
        Err(err) => Err(format!("Error while reading stdin {}", err)),
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
    println!("{} cause: {}", text, err);
    std::process::exit(1);
}
