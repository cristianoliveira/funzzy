use cli::errors::CliError;
use cli::Command;
use std::fs::File;
use std::io::prelude::*;

/// # Creates an funzzy yaml boilerplate.
/// 
pub struct InitCommand{
    folder_path: String,
}
impl InitCommand {
    pub fn new() -> Self {
        InitCommand{
            folder_path: String::from(".")
        }
    }
}
impl Command for InitCommand {
    fn execute(&self) -> Result<(), >{
        let mut yaml:File = try!(File::create("events.yaml"));
        yaml.write_all(format!("
        ## Funzzy events file
        # more details see: http://cristian.github.com/funzzy
        #
        # list here all the events and the commands that it should execute

        - when: '{}'
          change: 'echo \"It works!\"'
        ", self.folder_path).as_ref());
    }
}
