extern crate rustc_serialize;
extern crate docopt;

pub mod cli;
use docopt::Docopt;
use cli::Args;

const USAGE: &'static str = "
Funzzy the watcher.

Usage:
  funzzy watch
  funzzy init

Options:
  -h --help     Shows this message.
  -v --version  Shows version.
";

fn main() {
    let args: Args = Docopt::new(USAGE)
                            .and_then(|dopt| dopt.decode())
                            .unwrap_or_else(|e| e.exit());

    match cli::command(args) {
       Some(command) => match command.execute() {
           Ok(()) => println!("Command execute."),
           Err(err) => println!("{}", err)
       },
       None => println!("{}", USAGE)
    }
}

