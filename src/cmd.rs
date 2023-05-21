use std::process::{ Command };

pub fn execute(command_line: String) -> Result<(), String> {
    let shell = std::env::var("SHELL").unwrap_or(String::from("/bin/sh"));
    let mut cmd = Command::new(shell);
    match cmd.arg("-c").arg(command_line).status() {
        Err(error) => return Err(String::from(error.to_string())),
        Ok(status) => {
            if status.success() {
                return Ok(());
            } else {
                return Err(String::from("Command failed"));
            }
        }
    };
}

#[test]
fn it_executes_a_command() {
    let result = match execute(String::from("echo 'foo'")) {
        Ok(_) => true,
        Err(err) => panic!("{:?}",err),
    };

    assert!(result)
}
