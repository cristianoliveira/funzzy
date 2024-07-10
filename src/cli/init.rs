use crate::cli::Command;

use std::fs::File;
use std::io::Write;

pub const DEFAULT_CONTENT: &str = "## Funzzy events file
# more details see: https://github.com/cristianoliveira/funzzy
#
# List here the tasks and the commands for this workflow
# then run `fzz` to start to work.

- name: run my test
  run: 'ls -a'
  run_on_init: true
  change: '**/*.txt'
  ignore: '**/*.log'
";

/// # `InitCommand`
///
/// Creates a funzzy yaml boilerplate.
///
pub struct InitCommand {
    pub file_name: String,
}

impl InitCommand {
    pub fn new(file: &str) -> Self {
        InitCommand {
            file_name: file.to_string(),
        }
    }
}

impl Command for InitCommand {
    fn execute(&self) -> Result<(), String> {
        let res = match File::create(&self.file_name) {
            Ok(mut yaml) => {
                if let Err(err) = yaml.write_all(DEFAULT_CONTENT.as_ref()) {
                    return Err(err.to_string());
                }

                Ok(())
            }
            Err(err) => Err(format!("File wasn't created. Cause: {}", err)),
        };

        res?;

        Ok(())
    }
}
