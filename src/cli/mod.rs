pub mod init;
pub mod watch;

use std::error::Error;
use cli::init::InitCommand;
use cli::watch::WatchCommand;

#[derive(Debug, RustcDecodable)]
pub struct Args {
    // comands
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
    fn help(&self) -> &str;
}

/// # function command
///
/// Return a command based on [args] passed as param
/// or None if any command was found.
///
pub fn command(args: &Args) -> Option<Box<Command+'static>>{
    if args.cmd_init {
        return Some(Box::new(InitCommand));
    }
    if args.cmd_watch {
        return Some(Box::new(WatchCommand::new()));
    }
    None
}
