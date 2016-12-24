pub mod init;
pub mod watch;
pub mod exec;

pub use cli::init::InitCommand;
pub use cli::watch::{Watches, WatchCommand};
pub use cli::exec::ExecCommand;

/// # Command interface
///
/// Each command from cli should implement this.
///
pub trait Command {
    fn execute(&self) -> Result<(), String>;
}
