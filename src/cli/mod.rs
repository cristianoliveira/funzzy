pub mod init;
pub mod watch;

use cli::init::InitCommand;
use cli::watch::{ Watches, WatchCommand };
#[warn(unused_imports)]
use std::io::prelude::*;
use std::fs::File;

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
}

impl Args {
    pub fn new() -> Args {
        Args {
            cmd_init: false,
            cmd_watch: false,
            arg_command: vec![String::new()],
            flag_c: false,
            flag_h: false,
            flag_v: false,
        }
    }
}

/// # Command interface
///
/// Each command from cli should implement this.
///
pub trait Command {
    fn execute(&self) -> Result<(), &str>;
}

/// # function command
///
/// Return a command based on [args] passed as param
/// or None if any command was found.
///
pub fn command(args: &Args) -> Option<Box<Command+'static>>{
    match *args {
        Args { cmd_init: true, .. } =>
            Some(Box::new(InitCommand{ file_name: watch::FILENAME })),

        Args { cmd_watch: true, flag_c: false, .. } => {
            let mut file = match File::open(watch::FILENAME) {
                Ok(f) => f,
                Err(err) => panic!("File {} cannot be open. Cause: {}",
                                   watch::FILENAME, err)
            };
            let mut content = String::new();
            let _ = file.read_to_string(&mut content).unwrap();

            Some(Box::new(WatchCommand::new(Watches::from(&content))))
        },

        Args { cmd_watch: true, flag_c: true, .. } => {
            let command_args = args.arg_command.clone();
            Some(Box::new(WatchCommand::new(Watches::from_args(command_args))))
        }

        _ => None
    }
}
