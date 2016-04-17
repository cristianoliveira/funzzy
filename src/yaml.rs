extern crate yaml_rust;
extern crate glob;

use self::yaml_rust::{Yaml, YamlLoader};
use std::process::Command as ShellCommand;
use std::sync::mpsc::channel;
use self::glob::Pattern;

/// # matches
/// It check if a given yaml item matches with a given path
pub fn matches(item: &Yaml, path: &str) -> bool {
    match *item {
        Yaml::Array(ref items) => items.iter().any(|i| matches(&i, path)),
        Yaml::String(ref item) => pattern_for(item).matches(path),
        _ => false,
    }
}

fn pattern_for(pattern: &str) -> Pattern {
    Pattern::new(&format!("**/{}", pattern)).unwrap()
}

/// # extract_commands
/// It extract one or more shell commands from a given Yaml
pub fn extract_commands(item: &Yaml) -> Vec<ShellCommand> {
    match *item {
        Yaml::Array(ref items) => items.iter().map(|i| to_command(i)).collect(),
        ref item => vec![to_command(&item)],
    }
}

fn to_command(item: &Yaml) -> ShellCommand {
    let command = item.as_str().unwrap();
    let mut args: Vec<&str> = command.split(' ').collect();
    let cmd = args.remove(0);

    let mut shell = ShellCommand::new(cmd);
    shell.args(&args);
    shell
}
