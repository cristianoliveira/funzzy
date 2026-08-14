pub mod config;
pub mod control;
pub mod format;
pub mod init;
pub mod run;
pub mod watch;
pub mod watch_non_block;

pub use crate::cli::control::{ControlAction, ControlCommand, OutputFormat};
pub use crate::cli::init::InitCommand;
pub use crate::cli::run::RunCommand;
pub use crate::cli::watch::WatchCommand;
pub use crate::cli::watch_non_block::WatchNonBlockCommand;
use crate::errors::FzzError;

/// # Command interface
///
/// Each command from cli should implement this.
///
pub trait Command {
    fn execute(&self) -> Result<(), FzzError>;
}
