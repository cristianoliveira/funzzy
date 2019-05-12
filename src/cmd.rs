use std::error::Error;
use std::process::Command;

fn command_parser(command: String) -> Vec<Command> {
    let mut commands = vec![];
    let mut tokens: Vec<&str> = command.split(' ').collect();

    let init = tokens.remove(0);
    let mut command = Command::new(init);
    while tokens.len() > 0 {
        let token = tokens.remove(0);
        match token.clone() {
            "&&" => {
                commands.push(command);
                command = Command::new(tokens.remove(0));
            }
            _ => {
                command.arg(token);
            }
        }
    }

    commands.push(command);

    commands
}

pub fn execute(command_line: String) -> Result<(), String> {
    let commands = command_parser(command_line);

    for mut cmd in commands {
        if let Err(error) = cmd.status() {
            return Err(String::from(error.description()));
        }
    }

    return Ok(());
}

#[test]
fn it_executes_a_command() {
    let result = match execute(String::from("echo 'foo'")) {
        Ok(_) => true,
        Err(msg) => panic!(msg),
    };

    assert!(result)
}

#[test]
fn it_creates_commands() {
    let result = command_parser(String::from("cargo build"));

    let mut expected = Command::new("cargo");
    expected.arg("build");
    assert_eq!(format!("{:?}", expected), format!("{:?}", result[0]))
}

#[test]
fn it_creates_commands_with_more_than_one_arg() {
    let result = command_parser(String::from("cargo build --verbose"));

    let mut expected = Command::new("cargo");
    expected.arg("build");
    expected.arg("--verbose");
    assert_eq!(format!("{:?}", expected), format!("{:?}", result[0]))
}

#[test]
fn it_accept_nested_commands() {
    let result = command_parser(String::from("cargo build --verbose && cargo test"));

    let mut cmd1 = Command::new("cargo");
    cmd1.arg("build");
    cmd1.arg("--verbose");

    let mut cmd2 = Command::new("cargo");
    cmd2.arg("test");

    let commands = vec![cmd1, cmd2];

    assert_eq!(format!("{:?}", commands), format!("{:?}", result))
}
