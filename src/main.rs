// #![feature(plugin)]
// #![plugin(clippy)]
// #![deny(clippy)]

#[macro_use]
extern crate serde_derive;
extern crate docopt;

mod cli;
mod cmd;
mod rules;
mod yaml;

use std::fs::File;
use std::io;
#[warn(unused_imports)]
use std::io::prelude::*;

use cli::*;

use docopt::Docopt;

const VERSION: &'static str = env!("CARGO_PKG_VERSION");
const USAGE: &'static str = "
Funzzy the watcher.

Usage:
  funzzy
  funzzy init
  funzzy <command>
  funzzy watch [--verbose]
  funzzy watch [--verbose | -c | -s] <command>
  funzzy run [--verbose] <command> <interval>
  funzzy [options]

Options:
  run               Execute command in a given interval (seconds)
  -h --help         Shows this message.
  -v --version      Shows version.
  --verbose         Use verbose output.
  -c                Execute given command for current folder.
";

#[derive(Debug, Deserialize)]
pub struct Args {
    // comand
    pub cmd_init: bool,
    pub cmd_watch: bool,
    pub cmd_run: bool,

    pub arg_command: String,
    pub arg_interval: u64,

    // options
    pub flag_c: bool,
    pub flag_h: bool,
    pub flag_v: bool,
    pub flag_verbose: bool,
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
            cmd_watch: true,
            flag_c: true,
            ..
        } => {
            let watches = Watches::from_args(args.arg_command);
            execute(WatchCommand::new(watches, args.flag_verbose))
        }

        Args {
            ref arg_command, ..
        } if !arg_command.is_empty() => {
            match from_stdin() {
                Some(content) => {
                    let watches = Watches::new(rules::from_string(content, arg_command));
                    execute(WatchCommand::new(watches, args.flag_verbose));
                }
                None => show("Nothing to run"),
            };
        }

        _ => {
            let watches = Watches::from(&from_file(cli::watch::DEFAULT_FILENAME));
            execute(WatchCommand::new(watches, args.flag_verbose));
        }
    }
}

fn execute<T: Command>(command: T) {
    if let Err(err) = command.execute() {
        println!("Error: {}", err);
    }
}

fn from_stdin() -> Option<String> {
    let mut buffer = String::new();
    let stdin = io::stdin();
    let mut handle = stdin.lock();

    match handle.read_to_string(&mut buffer) {
        Ok(bytes) => {
            if bytes > 0 {
                Some(buffer)
            } else {
                None
            }
        }
        Err(err) => panic!("Error while reading stdin {}", err),
    }
}

fn from_file(filename: &str) -> String {
    let mut file = match File::open(filename) {
        Ok(f) => f,
        Err(err) => show(
            format!(
                "File {} cannot be opened. Cause: {}",
                cli::watch::DEFAULT_FILENAME,
                err
            )
            .as_str(),
        ),
    };

    let mut content = String::new();
    if let Err(err) = file.read_to_string(&mut content) {
        panic!("Error while trying read file. {}", err);
    }

    content
}

fn show(text: &str) -> ! {
    println!("{}", text);
    std::process::exit(0)
}
