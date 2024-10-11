use assert_cmd::Command;

const BINARY_NAME: &str = "funzzy";

#[test]
fn it_fails_when_config_file_alredy_exists() -> Result<(), Box<dyn std::error::Error>> {
    if !std::path::Path::new(".watch.yaml").exists() {
        return Err("Test pre-check: .watch.yaml doesn't exist".into());
    }

    let mut cmd = Command::cargo_bin(BINARY_NAME)?;
    cmd.arg("init").assert().failure().stdout(
        vec![
            "Execution failed",
            "  The command was not able to execute.",
            "  Reason: Configuration file already exists (.watch.yaml)",
            "",
        ]
        .join("\n"),
    );

    Ok(())
}
