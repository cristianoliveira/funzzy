extern crate funzzy;
mod cli;

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
