extern crate yaml_rust;
extern crate glob;

use self::yaml_rust::{Yaml, YamlLoader};
use std::process::Command as ShellCommand;
use std::process::exit;
use self::glob::Pattern;

/// # `matches`
///
/// It checks if a given yaml item matches with a given path
///
pub fn matches(item: &Yaml, path: &str) -> bool {
    match *item {
        Yaml::Array(ref items) => items.iter().any(|i| matches(i, path)),
        Yaml::String(ref item) => {
            pattern_for(item).matches(path)
        },
        _ => false,
    }
}

pub fn validate(yaml: &Yaml, key: &str) {
    if yaml[key].is_badvalue() {
        println!("File has a bad format. (Key {} not found)", key);
        exit(0)
    }
}

fn pattern_for(pattern: &str) -> Pattern {
    Pattern::new(&format!("**/{}", pattern)).expect("Pattern error.")
}

/// # `extract_commands`
///
/// It extracts one or more shell commands from a given Yaml
///
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

#[test]
fn it_matches() {
    let content = "
    - name: test
      path: ['tests/**', 'unit_tests/**']
    ";
    let yaml = YamlLoader::load_from_str(content).unwrap();
    assert!(self::matches(&yaml[0][0]["path"], "tests/main.rs"));
    assert!(self::matches(&yaml[0][0]["path"], "tests/cli/main.rs"));
    assert!(self::matches(&yaml[0][0]["path"], "tests/cli/other/main.rb"));
    assert!(self::matches(&yaml[0][0]["path"], "tests/other/main.rb"));
    assert!(self::matches(&yaml[0][0]["path"], "unit_tests/other/main.rb"))
}

#[test]
fn it_does_not_match() {
    let content = "
    - name: test
      path: tests/**
    ";
    let yaml = YamlLoader::load_from_str(content).unwrap();
    assert!(!self::matches(&yaml[0][0]["path"], "src/main.rs"));
    assert!(!self::matches(&yaml[0][0]["path"], "src/cli/main.rs"));
    assert!(!self::matches(&yaml[0][0]["path"], "src/cli/other/main.rb"));
    assert!(!self::matches(&yaml[0][0]["path"], "src/other/main.rb"))
}

#[test]
#[should_panic]
fn it_is_invalid_key_validation() {
    let content = "
    - name: test
      path: tests/**
    ";
    let yaml = YamlLoader::load_from_str(content).unwrap();
    self::validate(&yaml[0][0], "missing_key");
}

#[test]
fn it_is_valid_key_validation() {
    let content = "
    - name: test
      path: tests/**
    ";
    let yaml = YamlLoader::load_from_str(content).unwrap();
    self::validate(&yaml[0][0], "path");
}
