#[path = "./macros.rs"]
mod macros;

use crate::defer;

use std::{
    env,
    fs::File,
    process::{Command, Stdio},
};

pub struct Options {
    pub output_file: &'static str,
    pub example_file: &'static str,
}

pub fn with_example<F>(opts: Options, handler: F) -> ()
where
    F: FnOnce(&mut Command, File) -> (),
{
    let dir = env::current_dir().expect("error getting current directory");

    // Try to remove the file but ignore the error which is thrown if the file does not exist
    let _ = std::fs::remove_file(dir.join(opts.output_file));

    // check if the file exists if so fail
    assert!(
        !std::path::Path::new(&dir.join(opts.output_file)).exists(), 
       "the log file already exists, make sure to give an unique log file to avoid multiple writes to same file: {}",
       dir.join(opts.output_file).display()
    );

    let bin_path = dir.join("target/debug/fzz");
    let output_file = File::create(dir.join(opts.output_file)).expect("error log file");

    handler(
        Command::new(bin_path)
            .arg("-c")
            .arg(dir.join(opts.example_file))
            .stdout(Stdio::from(output_file)),
        File::open(dir.join(opts.output_file)).expect("failed to open file"),
    );

    std::fs::remove_file(dir.join(opts.output_file)).expect("failed to remove file after running test");
}
