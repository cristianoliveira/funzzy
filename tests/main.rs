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
   let mut args = funzzy::cli::Args::new();
   assert!(funzzy::cli::command(&args).is_none())
}

#[test]
fn it_returns_watch_command() {
   let mut args = funzzy::cli::Args::new();
   args.cmd_watch = true;
   assert!(funzzy::cli::command(&args).is_some())
}
