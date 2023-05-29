pub mod init;
pub mod run;
pub mod watch;
pub mod watch_non_block;

pub use self::run::RunCommand;
pub use cli::init::InitCommand;
pub use cli::watch::WatchCommand;
pub use cli::watch_non_block::WatchNonBlockCommand;

/// # Command interface
///
/// Each command from cli should implement this.
///
pub trait Command {
    fn execute(&self) -> Result<(), String>;
}
