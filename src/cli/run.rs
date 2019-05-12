use std::error::Error;
use std::process::Command as ShellCommand;
use std::{thread, time};

use cli::Command;

/// # `RunCommand`
///
/// Executes commands in an interval of time
///
pub struct RunCommand {
    command: String,
    interval: u64,
}

impl RunCommand {
    pub fn new(command: String, interval: u64) -> Self {
        RunCommand {
            command: command,
            interval: interval,
        }
    }
}

impl Command for RunCommand {
    fn execute(&self) -> Result<(), String> {
        let mut command_line: Vec<&str> = self.command.split_whitespace().collect();
        let mut command = ShellCommand::new(command_line.remove(0));
        command.args(&command_line);

        loop {
            if let Err(error) = command.status() {
                return Err(String::from(error.description()));
            }
            let wait = time::Duration::from_secs(self.interval);
            thread::sleep(wait)
        }
    }
}
