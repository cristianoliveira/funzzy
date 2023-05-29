use std::process::{Child, Command};
use std::sync::mpsc::channel;
use std::sync::mpsc::Sender;
use std::thread::JoinHandle;
use stdout;

pub fn execute(command_line: String) -> Result<(), String> {
    let shell = std::env::var("SHELL").unwrap_or(String::from("/bin/sh"));
    let mut cmd = Command::new(shell);
    match cmd.arg("-c").arg(command_line).status() {
        Err(error) => return Err(String::from(error.to_string())),
        Ok(status) => {
            if status.success() {
                return Ok(());
            } else {
                println!("This task command has failed {}", status);
                return Ok(());
            }
        }
    };
}

pub fn spawn_command(command_line: String) -> Result<Child, String> {
    let shell = std::env::var("SHELL").unwrap_or(String::from("/bin/sh"));
    let mut cmd = Command::new(shell);

    match cmd.arg("-c").arg(command_line).spawn() {
        Ok(child) => Ok(child),
        Err(err) => Err(format!("Error while creating command {}", err)),
    }
}

#[test]
fn it_spawn_a_command_returning_a_child_ref() {
    let result = match spawn_command(String::from("echo 'foo'")) {
        Ok(mut child) => child.wait().expect("fail to wait"),
        Err(err) => panic!("{:?}", err),
    };

    assert_eq!(format!("{}", result), "exit status: 0")
}

#[test]
fn it_executes_a_command() {
    let result = match execute(String::from("echo 'foo'")) {
        Ok(_) => true,
        Err(err) => panic!("{:?}", err),
    };

    assert!(result)
}
