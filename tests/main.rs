extern crate funzzy;
mod cli;

fn new_args() -> funzzy::cli::Args {
    funzzy::cli::Args {
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
   assert!(funzzy::cli::command(&args).is_some())
}

#[test]
fn it_as_default_returns_watch_command() {
   let args = new_args();
   assert!(funzzy::cli::command(&args).is_some());
}

#[test]
fn it_returns_watch_command() {
   let mut args = new_args();
   args.cmd_watch = true;
   assert!(funzzy::cli::command(&args).is_some())
}

#[test]
fn it_returns_watch_command_with_arbitrary_command() {
   let mut args = new_args();
   args.cmd_watch = true;
   args.flag_c = true;
   args.arg_command = vec![String::from("cargo build")];
   assert!(funzzy::cli::command(&args).is_some())
}
