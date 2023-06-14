use crate::stdout;
use std::process::{Child, Command};

pub fn execute(command: &String) -> Result<(), String> {
    println!();
    stdout::info(&format!("{} \n", String::from(command)));

    let shell = std::env::var("SHELL").unwrap_or(String::from("/bin/sh"));
    let mut cmd = Command::new(shell);
    match cmd.arg("-c").arg(command).status() {
        Err(error) => Err(format!("Command {} has errored with {}", command, error)),
        Ok(status) => {
            if status.success() {
                Ok(())
            } else {
                Err(format!("Command {} has failed with {}", command, status))
            }
        }
    }
}

pub fn spawn(command: &String) -> Result<Child, String> {
    println!();
    stdout::info(&format!("task {} \n", String::from(command)));

    let shell = std::env::var("SHELL").unwrap_or(String::from("/bin/sh"));
    let mut cmd = Command::new(shell);

    match cmd.arg("-c").arg(command).spawn() {
        Ok(child) => Ok(child),

        Err(error) => Err(format!("Command {} has errored with {}", command, error)),
    }
}

#[test]
fn it_spawn_a_command_returning_a_child_ref() {
    let result = match spawn(&String::from("echo 'foo'")) {
        Ok(mut child) => child.wait().expect("fail to wait"),
        Err(err) => panic!("{:?}", err),
    };

    assert_eq!(format!("{}", result), "exit status: 0")
}

#[test]
fn it_executes_a_command() {
    let result = match execute(&String::from("echo 'foo'")) {
        Ok(_) => true,
        Err(err) => panic!("{:?}", err),
    };

    assert!(result)
}
