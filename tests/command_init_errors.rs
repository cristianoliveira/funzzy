use assert_cmd::Command;

#[path = "./common/lib.rs"]
mod setup;

const BINARY_NAME: &str = "funzzy";

#[test]
fn it_fails_when_config_file_alredy_exists() -> Result<(), Box<dyn std::error::Error>> {
    setup::nonparallel(|| {
        if !std::path::Path::new(".watch.yaml").exists() {
            let mut cmd = Command::cargo_bin(BINARY_NAME).expect("failed to get cargo bin");
            cmd.arg("init").assert().success();
        }
        defer!({
            if std::path::Path::new("delete.txt").exists() {
                std::fs::remove_file(".watch.yaml").expect("failed to remove file");
            }
        });

        let mut cmd = Command::cargo_bin(BINARY_NAME)?;
        cmd.arg("init").assert().failure().stdout(
            vec![
                "Error: Command failed to execute",
                "Configuration file already exists (.watch.yaml)",
                "",
            ]
            .join("\n"),
        );

        Ok(())
    })
}

#[test]
fn it_fails_folder_is_read_only() -> Result<(), Box<dyn std::error::Error>> {
    setup::nonparallel(|| {
        std::env::set_current_dir("examples/workdir/init").expect("failed to change dir");
        //delete files in the folder
        std::fs::remove_file(".watch.yaml").expect("failed to remove .watch.yaml file");

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

        cmd.arg("init").assert().failure().stdout(
            vec![
                "Error: Command failed to execute",
                "Failed to create the configuration file",
                "Reason: Permission denied (os error 13)",
                "Hint: Check if you have permission to write in the current folder",
                "",
            ]
            .join("\n"),
        );

        Ok(())
    })
}
