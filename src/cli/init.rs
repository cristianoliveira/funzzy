use cli::Command;
use std::fs::File;
use std::io::prelude::*;

/// # InitCommand
/// Creates a funzzy yaml boilerplate.
/// 
pub struct InitCommand {
    pub file_name: &'static str
}
impl InitCommand {
    pub fn new() -> Self {
        InitCommand {
            file_name: "watch.yaml"
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
           Err(err) => panic!("Cannot write file.")
        }
    }

    fn help(&self) -> &str {
        " Init comand
        It creates a funzzy a boilerplate yaml
        "
    }
}
