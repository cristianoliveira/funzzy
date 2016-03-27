pub mod init;
pub mod watch;

use std::error::Error;
use cli::init::InitCommand;
use cli::watch::WatchCommand;
use std::io::prelude::*;
use std::fs::File;

#[derive(Debug, RustcDecodable)]
pub struct Args {
    // comand
    pub cmd_init: bool,
    pub cmd_watch: bool,

    // options
    pub flag_h: bool,
    pub flag_v: bool,
}

impl Args {
    pub fn new() -> Args {
        Args {
            cmd_init: false,
            cmd_watch: false,
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
    if args.cmd_init {
        return Some(Box::new(InitCommand::new()));
    }
    if args.cmd_watch {
        let mut file = File::open("watch.yaml").unwrap();
        let mut content = String::new();
        let _ = file.read_to_string(&mut content).unwrap();
        return Some(Box::new(WatchCommand::new(&content)));
    }
    None
}
