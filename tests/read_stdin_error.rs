use assert_cmd::Command;

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
