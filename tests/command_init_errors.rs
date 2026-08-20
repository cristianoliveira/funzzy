use assert_cmd::cargo;

#[path = "./common/lib.rs"]
mod setup;

#[test]
fn it_fails_when_config_file_alredy_exists() -> Result<(), Box<dyn std::error::Error>> {
    setup::nonparallel(|| {
        if !std::path::Path::new(".watch.yaml").exists() {
            let mut cmd = cargo::cargo_bin_cmd!("funzzy");
            cmd.arg("init").assert().success();
        }
        defer!({
            if std::path::Path::new("delete.txt").exists() {
                std::fs::remove_file(".watch.yaml").expect("failed to remove file");
            }
        });

        let mut cmd = cargo::cargo_bin_cmd!("funzzy");
        cmd.env("FUNZZY_COLORED", "true")
            .env("_TEST_FUNZZY_COLORED", "true")
            .arg("init")
            .assert()
            .failure()
            .stdout(
                [
                    "\u{1b}[31mError\u{1b}[0m: Command failed to execute",
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
        // Use a per-process temp dir instead of the shared
        // `examples/workdir/init` fixture: overlapping `cargo test`
        // invocations (watcher generations) run this binary concurrently in
        // separate processes, and the in-process mutex cannot serialize
        // them. A shared dir let one process's cleanup (restoring write
        // permissions, or creating `.watch.yaml`) land between the other's
        // `set_readonly` and `init` spawn, making `init` succeed or report
        // the wrong error.
        let dir = std::env::temp_dir().join(format!("funzzy-init-readonly-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).expect("failed to create test dir");
        let original_dir = std::env::current_dir().expect("failed to get current dir");
        std::env::set_current_dir(&dir).expect("failed to change dir");
        //delete files in the folder
        std::fs::remove_file(".watch.yaml").unwrap_or_default();

        let folder = std::fs::metadata(".").expect("failed to get metadata");
        let mut readonly = folder.permissions();
        readonly.set_readonly(true);
        std::fs::set_permissions(".", readonly).expect("failed to set read only");
        defer!({
            // Restore the folder permissions first (so it can be removed),
            // then remove it, then restore the process cwd: sibling tests
            // in this binary rely on running from the repo root.
            std::fs::set_permissions(".", folder.permissions())
                .expect("failed to restore folder permissions");
            let _ = std::fs::remove_dir_all(&dir);
            std::env::set_current_dir(&original_dir).expect("failed to restore dir");
        });

        let mut cmd = cargo::cargo_bin_cmd!("funzzy");

        cmd.env("FUNZZY_COLORED", "false")
            .env("_TEST_FUNZZY_COLORED", "false")
            .arg("init")
            .assert()
            .failure()
            .stdout(
                [
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
