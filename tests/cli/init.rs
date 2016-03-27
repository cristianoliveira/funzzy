extern crate funzzy;

use funzzy::cli::Command;
use funzzy::cli::init::InitCommand;
use std::fs::remove_file;
use std::path::Path;
use std::io::prelude::*;

#[test]
fn it_creates_new_config_file() {
    let file = "events.yaml";
    remove_file(file);

    let mut command = InitCommand::new();
    command.file_name = &file;
    let _ = command.execute();

    assert!(Path::new(file).exists());

    remove_file(file);
}
