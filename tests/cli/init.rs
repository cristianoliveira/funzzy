extern crate funzzy;

use funzzy::cli::Command;
use funzzy::cli::init::InitCommand;
use std::fs::remove_file;
use std::path::Path;

#[test]
fn it_creates_new_config_file() {
    let file = "events.yaml";
    let _ = remove_file(file);

    let mut command = InitCommand { file_name: "" };
    command.file_name = &file;
    let _ = command.execute();

    assert!(Path::new(file).exists());

    let _ = remove_file(file);
}
