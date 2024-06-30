#[path = "./macros.rs"]
mod macros;

use crate::defer;

use std::{
    env,
    fs::File,
    process::{Command, Stdio},
};

pub struct Options {
    pub log_file: &'static str,
    pub example_file: &'static str,
}

pub fn with_example<F>(opts: Options, handler: F) -> ()
where
    F: FnOnce(&mut Command, File) -> (),
{
    let dir = env::current_dir().expect("error getting current directory");

    // check if the file exists if so fail
    assert!(
        !std::path::Path::new(&dir.join(opts.log_file)).exists(), 
       "the log file already exists, make sure to give an unique log file to avoid multiple writes to same file: {}",
       dir.join(opts.log_file).display()
    );

    defer!({
        std::fs::remove_file(dir.join(opts.log_file)).expect("failed to remove test output file")
    });

    let bin_path = dir.join("target/debug/fzz");
    let _ = std::fs::remove_file(dir.join(opts.log_file));
    let output_log = File::create(dir.join(opts.log_file)).expect("error log file");

    handler(
        Command::new(bin_path)
            .arg("-c")
            .arg(dir.join(opts.example_file))
            .stdout(Stdio::from(output_log)),
        File::open(dir.join(opts.log_file)).expect("failed to open file"),
    );
}
