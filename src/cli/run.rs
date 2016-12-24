
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
        RunCommand { command: command, interval: interval }
    }
}

impl Command for RunCommand {
    fn execute(&self) -> Result<(), String> {
        let mut command_line: Vec<&str> = self.command.split_whitespace().collect();
        println!("{:?}", command_line);
        let mut command = ShellCommand::new(command_line.remove(0));
        command.args(&command_line);

        loop {
            println!("{:?}", command);
            if let Err(error) = command.status() {
                println!("{}", error);
                return Err(String::from(error.description()));
            }
            let wait = time::Duration::from_secs(self.interval);
            thread::sleep(wait)
        }
    }
}
