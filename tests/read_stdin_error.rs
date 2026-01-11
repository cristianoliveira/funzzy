use assert_cmd::cargo;
use predicates::prelude::predicate;
use std::process::{Command as StdCommand, Stdio};

#[path = "./common/lib.rs"]
mod setup;

#[test]
fn it_fails_when_no_stdin_is_given() -> Result<(), Box<dyn std::error::Error>> {
    let mut cmd = cargo::cargo_bin_cmd!("funzzy");

    cmd.env("FUNZZY_COLORED", "false")
        .env("FUNZZY_STDIN_TIMEOUT_MS", "10")
        .current_dir("examples/workdir")
        .arg("echo 'foo'")
        .write_stdin("")
        .assert()
        .failure()
        .stdout(
            vec![
                "Error: No files provided via stdin.",
                "Provide a list of files or directories via stdin, e.g., `find . | fzz 'echo {{filepath}}'`.",
                "",
            ]
            .join("\n"),
        );

    Ok(())
}

#[test]
fn it_validates_when_given_list_of_paths_is_invalid() -> Result<(), Box<dyn std::error::Error>> {
    let mut cmd = cargo::cargo_bin_cmd!("funzzy");
    let lsla = StdCommand::new("ls")
        .arg("-la")
        .arg("examples/workdir/ignored")
        .stdout(Stdio::piped())
        .output()?;

    cmd.env("FUNZZY_COLORED", "false")
        .env("FUNZZY_STDIN_TIMEOUT_MS", "10")
        .current_dir("examples/workdir")
        .arg("echo 'foo'")
        .write_stdin(lsla.stdout)
        .assert()
        .failure()
        .stdout(predicate::str::contains(
            vec![
                "Error: Failed to get rules from stdin",
                "Unknown path \'total", //... 8' line 1
            ]
            .join("\n"))
        )
        .stdout(predicate::str::contains(
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
