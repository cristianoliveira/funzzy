use cli::Command;

#[warn(unused_imports)]
use std::io::prelude::*;
use std::fs::File;

/// # InitCommand
/// Creates a funzzy yaml boilerplate.
/// 
pub struct InitCommand {
    pub file_name: &'static str
}
impl InitCommand {
    pub fn new() -> Self {
        InitCommand {
            file_name: ""
        }
    }
}

impl Command for InitCommand {
    fn execute(&self) -> Result<(), &str>{
        const DEFAULT_CONTENT: &'static str =
"## Funzzy events file
 # more details see: http://cristian.github.com/funzzy
 #
 # list here all the events and the commands that it should execute

- name: run my tests
  when:
    change: 'src/**'
    run: ls -a
";

        let mut yaml:File = match File::create(self.file_name) {
           Ok(f) => f,
           Err(err) => panic!("File not created. Cause: {}", err)
        };

        match yaml.write_all(DEFAULT_CONTENT.as_ref()) {
           Ok(_) => Ok(()),
           Err(_) => panic!("Cannot write file.")
        }
    }
}
