// #![feature(plugin)]
// #![plugin(clippy)]
// #![deny(clippy)]

extern crate rustc_serialize;
extern crate docopt;

mod cli;
mod yaml;

use docopt::Docopt;
use cli::{Args, USAGE};

const VERSION: &'static str = env!("CARGO_PKG_VERSION");

fn main() {
    let args: Args = Docopt::new(USAGE)
        .and_then(|dopt| dopt.decode())
        .unwrap_or_else(|e| e.exit());

    println!("{:?}",args);

    match args {
        Args { flag_v: true, .. } => show(VERSION),
        Args { flag_h: true, .. } => show(USAGE),
        _ => {
            if let Some(command) = cli::command(&args) {
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

