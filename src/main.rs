// #![feature(plugin)]
// #![plugin(clippy)]
// #![deny(clippy)]

extern crate rustc_serialize;
extern crate docopt;

mod cli;
mod yaml;

#[warn(unused_imports)]
use std::io::prelude::*;
use std::fs::File;

use cli::{InitCommand, Watches, WatchCommand};

use docopt::Docopt;

const VERSION: &'static str = env!("CARGO_PKG_VERSION");
const USAGE: &'static str = "
Funzzy the watcher.

Usage:
  funzzy
  funzzy watch [--verbose]
  funzzy watch [--verbose | -c | -s] <command>
  funzzy init
  funzzy [options]

Options:
  -h --help         Shows this message.
  -v --version      Shows version.
  --verbose         Use verbose output.
  -c                Execute given command for current folder.
";

#[derive(Debug, RustcDecodable)]
pub struct Args {
    // comand
    pub cmd_init: bool,
    pub cmd_watch: bool,
    pub arg_command: Vec<String>,

    // options
    pub flag_c: bool,
    pub flag_h: bool,
    pub flag_v: bool,
    pub flag_verbose: bool,
}

fn main() {
    let args: Args = Docopt::new(USAGE)
        .and_then(|dopt| dopt.decode())
        .unwrap_or_else(|e| e.exit());

    match args {
        Args { flag_v: true, .. } => show(VERSION),
        Args { flag_h: true, .. } => show(USAGE),
        _ => {
            if let Some(command) = command(&args) {
                if let Err(err) = command.execute() {
                    println!("Error: {}", err)
                }
            }
        }
    }
}

fn show(text: &str) -> ! {
    println!("{}", text);
    std::process::exit(0)
}

/// # command
///
/// Returns a command based on [args] passed as param
///
pub fn command(args: &Args) -> Option<Box<cli::Command + 'static>> {
    match *args {
        Args { cmd_init: true, .. } =>
            Some(Box::new(InitCommand { file_name: cli::watch::FILENAME })),

        Args { cmd_watch: true, flag_c: true, .. } => {
            let command_args = args.arg_command.clone();
            let watches = Watches::from_args(command_args);
            watches.validate();
            Some(Box::new(WatchCommand::new(watches, args.flag_verbose)))
        }

        _ => {
            let mut file = match File::open(cli::watch::FILENAME) {
                Ok(f) => f,
                Err(err) =>
                    panic!("File {} cannot be open. Cause: {}",
                           cli::watch::FILENAME, err),
            };

            let mut content = String::new();
            if let Err(err) = file.read_to_string(&mut content) {
                panic!("Error while trying read file. {}",err);
            }

            let watches = Watches::from(&content);
            watches.validate();
            Some(Box::new(WatchCommand::new(watches, args.flag_verbose)))
        }
    }
}

#[cfg(test)]
mod tests {
    extern crate notify;
    extern crate yaml_rust;
    extern crate glob;

    use super::*;

    fn new_args() -> Args {
        Args {
            cmd_init: false,
            cmd_watch: false,
            arg_command: vec![String::new()],
            flag_c: false,
            flag_h: false,
            flag_v: false,
            flag_verbose: false,
       }
    }

    #[test]
    fn it_returns_some_command() {
       let mut args = new_args();
       args.cmd_init = true;
       assert!(command(&args).is_some())
    }

    #[test]
    fn it_returns_watch_command_by_default() {
       let args = new_args();
       assert!(command(&args).is_some());
    }

    #[test]
    fn it_returns_watch_command() {
       let mut args = new_args();
       args.cmd_watch = true;
       assert!(command(&args).is_some())
    }

    #[test]
    fn it_returns_watch_command_with_arbitrary_command() {
       let mut args = new_args();
       args.cmd_watch = true;
       args.flag_c = true;
       args.arg_command = vec![String::from("cargo build")];
       assert!(command(&args).is_some())
    }

}
