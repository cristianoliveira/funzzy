use assert_cmd::cargo;

#[path = "./common/lib.rs"]
mod setup;

fn change_dir_if_needed() {
    let current_dir = std::env::current_dir().expect("failed to get current dir");
    if !current_dir.ends_with("examples/workdir/watch") {
        std::env::set_current_dir("examples/workdir/watch").ok();
    }
}

#[test]
fn it_fails_when_folder_is_read_only() -> Result<(), Box<dyn std::error::Error>> {
    change_dir_if_needed();

    let mut cmd = cargo::cargo_bin_cmd!("funzzy");
    cmd.env("FUNZZY_COLORED", "false");
    cmd.assert().failure().stdout(
        vec![
            "Error: Failed to read default config file",
            "Couldn\'t open configuration file: \'.watch.yaml\'",
            "Reason: No such file or directory (os error 2)",
            "Hint: Check if the file exists and if the path is correct. Try `fzz init` to create a new configuration file",
            "",
        ]
        .join("\n"),
    );

    Ok(())
}

#[test]
fn it_fails_using_an_config_with_missing_properties() -> Result<(), Box<dyn std::error::Error>> {
    change_dir_if_needed();

    let mut cmd = cargo::cargo_bin_cmd!("funzzy");
    cmd.env("FUNZZY_COLORED", "false");
    cmd.arg("--config")
        .arg("./missing-required-property.yml")
        .assert()
        .failure()
        .stdout(
            vec![
                "Error: Failed to read config file",
                "Missing \'name\' in rule",
                "```yaml",
                "foo: bla",
                "run: bar",
                "```",
                "Hint: Check for typos or wrong identation",
                "",
            ]
            .join("\n"),
        );

    Ok(())
}

#[test]
fn it_fails_using_an_config_with_non_list() -> Result<(), Box<dyn std::error::Error>> {
    change_dir_if_needed();

    let mut cmd = cargo::cargo_bin_cmd!("funzzy");
    cmd.env("FUNZZY_COLORED", "false");
    cmd.arg("--config")
        .arg("./non-list.yaml")
        .assert()
        .failure()
        .stdout(
            vec![
                "Error: Failed to read config file",
                "Configuration file is invalid. Expected an Array/List of rules got: Hash",
                "```yaml",
                "on:",
                "  - name: foobar",
                "    run: echo hello",
                "    change: hello/*",
                "```",
                "Hint: Make sure to declare the rules as a list without any root property",
                "",
            ]
            .join("\n"),
        );

    Ok(())
}
