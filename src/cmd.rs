use std::process::{Child, Command};

pub fn execute(command_line: &String) -> Result<(), String> {
    let shell = std::env::var("SHELL").unwrap_or(String::from("/bin/sh"));
    let mut cmd = Command::new(shell);
    match cmd.arg("-c").arg(command_line.to_owned()).status() {
        Err(error) => Err(format!(
            "Command {} has errored with {}",
            command_line, error
        )),
        Ok(status) => {
            if status.success() {
                Ok(())
            } else {
                Err(format!(
                    "Command {} has failed with {}",
                    command_line, status
                ))
            }
        }
    }
}

pub fn spawn_command(command_line: String) -> Result<Child, String> {
    let shell = std::env::var("SHELL").unwrap_or(String::from("/bin/sh"));
    let mut cmd = Command::new(shell);

    match cmd.arg("-c").arg(command_line.to_owned()).spawn() {
        Ok(child) => Ok(child),

        Err(error) => Err(format!(
            "Command {} has errored with {}",
            command_line, error
        )),
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
    let result = match execute(&String::from("echo 'foo'")) {
        Ok(_) => true,
        Err(err) => panic!("{:?}", err),
    };

    assert!(result)
}
