pub mod init;
pub mod watch;
pub mod run;
pub mod rules;

pub use cli::init::InitCommand;
pub use cli::watch::{Watches, WatchCommand};
pub use cli::run::RunCommand;
pub use cli::rules::Rules;

/// # Command interface
///
/// Each command from cli should implement this.
///
pub trait Command {
    fn execute(&self) -> Result<(), String>;
}
