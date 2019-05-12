pub mod init;
pub mod run;
pub mod watch;

pub use self::run::RunCommand;
pub use cli::init::InitCommand;
pub use cli::watch::{WatchCommand, Watches};

/// # Command interface
///
/// Each command from cli should implement this.
///
pub trait Command {
    fn execute(&self) -> Result<(), String>;
}
