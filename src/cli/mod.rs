pub mod errors;
pub mod init;

use cli::errors::CliError;
use cli::init::InitCommand;

#[derive(Debug, RustcDecodable)]
pub struct Args {
    pub cmd_init: bool,
    pub arg_folder: Option<String>,
    pub flag_h: bool,
    pub flag_v: bool,
}

impl Args {
    pub fn new() -> Args {
        Args {
            cmd_init: false,
            arg_folder: None,
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
    fn execute(&self) -> Result<(), CliError>;
}

/// # function command
///
/// Return a command based on [args] passed as param
/// or None if any command was found.
///

pub fn execute(args: Args) -> Option<Box<Command+'static>>{
    if args.cmd_init {
        return Some(Box::new(InitCommand));
    }
    None
}


