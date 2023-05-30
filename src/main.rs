// #![feature(plugin)]
// #![plugin(clippy)]
// #![deny(clippy)]

#[macro_use]
extern crate serde_derive;
extern crate docopt;

mod cli;
mod cmd;
mod rules;
mod stdout;
mod watches;
mod yaml;

use cli::*;
use watches::Watches;

use std::io;
#[warn(unused_imports)]
use std::io::prelude::*;

use docopt::Docopt;

const VERSION: &str = env!("CARGO_PKG_VERSION");
const USAGE: &str = "
Funzzy the watcher.

Usage:
  funzzy [options]
  funzzy init
  funzzy watch [<command>] [options]
  funzzy run <command> <interval>
  funzzy <command> [options]

Commands:
    init                Create a new funzzy.yml file.
    watch               Watch for file changes and execute a command.
    run                 Run a command every <interval> seconds.

Options:
  <command>            Run an arbitrary command for current folder.
  --config=<cfgfile>   Use given config file.
  --target=<task>      Execute only the given task target.
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
    pub cmd_run: bool,

    pub arg_command: String,
    pub arg_interval: u64,

    // options
    pub flag_config: String,
    pub flag_target: String,

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
        Args { flag_v: true, .. } => show(VERSION),
        Args { flag_h: true, .. } => show(USAGE),

        // Commands
        Args { cmd_init: true, .. } => execute(InitCommand::new(cli::watch::DEFAULT_FILENAME)),

        Args { cmd_run: true, .. } => execute(RunCommand::new(args.arg_command, args.arg_interval)),

        Args {
            ref arg_command, ..
        } if !arg_command.is_empty() => {
            match from_stdin() {
                Ok(content) => {
                    let watches = Watches::new(rules::from_string(content, arg_command));
                    execute(WatchCommand::new(watches, args.flag_V));
                }
                Err(err) => show(err.as_str()),
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
                            stdout::info(&format!("No target found for {}", args.flag_target));
                            stdout::info(&format!("Available targets:"));

                            for rule in rules {
                                stdout::info(&format!("  {}", rule.name));
                            }

                            show("Finished there is no task to run");
                        } else {
                            execute(WatchCommand::new(Watches::new(filtered), args.flag_V));
                        }
                    } else {
                        execute(WatchCommand::new(Watches::new(rules), args.flag_V));
                    }
                }
                Err(err) => error("Error while reading config file", err),
            }
        }
    }
}

fn execute<T: Command>(command: T) {
    if let Err(err) = command.execute() {
        stdout::error(&format!("{}", err));
    }
}

fn from_stdin() -> Result<String, String> {
    let mut buffer = String::new();
    let stdin = io::stdin();
    let mut handle = stdin.lock();

    match handle.read_to_string(&mut buffer) {
        Ok(bytes) => {
            if bytes > 0 {
                Ok(buffer)
            } else {
                Err(String::from("No input"))
            }
        }
        Err(err) => Err(format!("Error while reading stdin {}", err)),
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
