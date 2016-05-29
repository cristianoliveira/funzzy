pub mod init;
pub mod watch;

pub use cli::init::InitCommand;
pub use cli::watch::{Watches, WatchCommand};
#[warn(unused_imports)]
use std::io::prelude::*;
use std::fs::File;

/// # Command interface
///
/// Each command from cli should implement this.
///
pub trait Command {
    fn execute(&self) -> Result<(), String>;
}
