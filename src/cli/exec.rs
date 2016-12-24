
extern crate notify;
extern crate yaml_rust;
extern crate glob;

use std::{thread, time};
use std::process::Command as ShellCommand;
use std::error::Error;

use self::notify::{RecommendedWatcher, Watcher};
use self::yaml_rust::{Yaml, YamlLoader};

use cli::Command;
use yaml;


pub const FILENAME: &'static str = ".watch.yaml";

/// # `ExecCommand`
///
/// Executes commands in an interval of time
///
pub struct ExecCommand {
    command: String,
    interval: u64,
}

impl ExecCommand {
    pub fn new(command: String, interval: u64) -> Self {
        ExecCommand { command: command, interval: interval }
    }
}

impl Command for ExecCommand {
    fn execute(&self) -> Result<(), String> {
        let mut command = ShellCommand::new(&self.command);
        loop {
            if let Err(error) = command.status() {
                return Err(String::from(error.description()));
            }
            let wait = time::Duration::from_secs(self.interval);
            thread::sleep(wait)
        }
    }
}
