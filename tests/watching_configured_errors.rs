use assert_cmd::Command;

#[path = "./common/lib.rs"]
mod setup;

const BINARY_NAME: &str = "funzzy";

#[test]
fn it_fails_when_folder_is_read_only() -> Result<(), Box<dyn std::error::Error>> {
    setup::nonparallel(|| {
        std::env::set_current_dir("examples/workdir/init").expect("failed to change dir");

        let folder = std::fs::metadata(".").expect("failed to get metadata");
        let mut readonly = folder.permissions();
        readonly.set_readonly(true);
        std::fs::set_permissions(".", readonly).expect("failed to set read only");
        defer!({
            let mut perms = folder.permissions();
            perms.set_readonly(false);
            std::fs::set_permissions(".", perms).expect("failed to set read only");
        });

        let mut cmd = Command::cargo_bin(BINARY_NAME)?;
        cmd.assert().failure().stdout(
            vec![
                "Error: Failed to read default config file",
                "Couldn\'t open configuration file: \'.watch.yaml\'",
                "Reason: No such file or directory (os error 2)",
                "Hint: Check if the file exists and if the path is correct",
                "",
            ]
            .join("\n"),
        );

        Ok(())
    })
}
