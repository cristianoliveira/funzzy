extern crate funzzy;
mod cli;

#[warn(unused_imports)]
use std::io::prelude::*;
use std::fs::{ File, remove_file };

#[test]
fn it_returns_some_command() {
   let mut args = funzzy::cli::Args::new();
   args.cmd_init = true;
   assert!(funzzy::cli::command(&args).is_some())
}

#[test]
fn it_returns_no_command() {
   let args = funzzy::cli::Args::new();
   assert!(funzzy::cli::command(&args).is_none())
}

#[test]
fn it_returns_watch_command() {
   setup();
   let mut args = funzzy::cli::Args::new();
   args.cmd_watch = true;
   assert!(funzzy::cli::command(&args).is_some())
}

#[test]
fn it_returns_watch_command_with_arbitrary_command() {
   let mut args = funzzy::cli::Args::new();
   args.cmd_watch = true;
   args.flag_c = true;
   args.arg_command = vec![String::from("cargo build")];
   assert!(funzzy::cli::command(&args).is_some())
}

#[test]
#[should_panic(expected = "File .watch.yaml cannot be open. Cause: No such file or directory (os error 2)")]
fn it_raise_panic_when_file_not_found() {
   setup();
   let _ = remove_file(".watch.yaml");
   let mut args = funzzy::cli::Args::new();
   args.cmd_watch = true;
   assert!(funzzy::cli::command(&args).is_some())
}

fn setup() {
    let mut file: File = File::create(".watch.yaml").unwrap();
    file.write_all(funzzy::cli::init::DEFAULT_CONTENT.as_ref());
}
