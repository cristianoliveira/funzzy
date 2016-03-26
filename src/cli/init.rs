use cli::errors::CliError;
use cli::Command;
use std::fs::File;
use std::io::prelude::*;

/// # Creates an funzzy yaml boilerplate.
/// 
pub struct InitCommand;
impl Command for InitCommand {
    fn execute(&self) -> Result<(), CliError>{
        const DEFAULT_CONTENT: &'static str = "
 ## Funzzy events file
 # more details see: http://cristian.github.com/funzzy
 #
 # list here all the events and the commands that it should execute

 - name: run my tests
   when: '.'
   do: 'echo \"It works!\"' \0";

        let mut yaml:File = try!(File::create("events.yaml"));

        try!(yaml.write_all(DEFAULT_CONTENT.as_ref()));
        Ok(())
    }
}
