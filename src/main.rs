extern crate rustc_serialize;
extern crate docopt;
extern crate yaml_rust;

mod cli;

use docopt::Docopt;
use yaml_rust::{YamlLoader, YamlEmitter};
use cli::Args;

const VERSION: &'static str = env!("CARGO_PKG_VERSION");
const USAGE: &'static str = "
Funzzy the watcher.

Usage:
  funzzy watch
  funzzy init
  funzzy -h
  funzzy -v

Options:
  -h --help         Shows this message.
  -v --version      Shows version.
";

fn main() {
    let args: Args = Docopt::new(USAGE)
                            .and_then(|dopt| dopt.decode())
                            .unwrap_or_else(|e| e.exit());

    match args {
        Args { flag_v: true, .. } => show(VERSION),
        Args { flag_h: true, .. } => show(USAGE),
        _ => match cli::command(&args) {
           Some(command) =>
              match command.execute() { 
                  Ok(()) => println!("Command execute."),
                  Err(err) => println!("{}", err)
              },
           None => show(USAGE)
        }
    }
}

fn show(text: &str) -> ! {
    println!("{}", text);
    std::process::exit(0)
}
