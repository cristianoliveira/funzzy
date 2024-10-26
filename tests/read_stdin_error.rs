use assert_cmd::Command;
use std::process::{Command as StdCommand, Stdio};

#[path = "./common/lib.rs"]
mod setup;

const BINARY_NAME: &str = "funzzy";

#[test]
fn it_fails_when_no_stdin_is_given() -> Result<(), Box<dyn std::error::Error>> {
    let mut cmd = Command::cargo_bin(BINARY_NAME)?;

    cmd.arg("echo 'foo'")
        .write_stdin("")
        .assert()
        .failure()
        .stdout(
            vec![
                "Error: Failed to read stdin",
                "Reason: Timed out waiting for input.",
                "Hint: Did you forget to pipe an output of a command? Try `find . | fzz 'echo \"changed: {{filepath}}\"'`",
                "",
            ]
            .join("\n"),
        );

    Ok(())
}

#[test]
fn it_validates_when_given_list_of_paths_is_invalid() -> Result<(), Box<dyn std::error::Error>> {
    let mut cmd = Command::cargo_bin(BINARY_NAME)?;
    let lsla = StdCommand::new("ls")
        .arg("-la")
        .arg("examples/workdir/ignored")
        .stdout(Stdio::piped())
        .output()?;

    cmd.arg("echo 'foo'")
        .write_stdin(lsla.stdout)
        .assert()
        .failure()
        .stdout(predicates::str::contains(
            vec![
                "Error: Failed to get rules from stdin",
                "Unknown path \'total", //... 8' line 1
            ]
            .join("\n"))
        )
        .stdout(predicates::str::contains(
            vec![
                "Reason: No such file or directory (os error 2)",
                "Hint: When using stdin, make sure to provide a list of valid files or directories.",
                "The output of command `find` is a good example",
                "",
            ]
            .join("\n")),
        );

    Ok(())
}
