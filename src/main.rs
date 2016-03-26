extern crate rustc_serialize;
extern crate docopt;

pub mod cli;

use std::fs::{OpenOptions};
use std::io::prelude::*;
use docopt::Docopt;

const USAGE: &'static str = "
Funzzy the watcher.

Usage:
  funzzy watch
  funzzy init

Options:
  -h --help Shows this message.
";

fn main() {

    println!("Hello, world!");
}

