use cli::Command;

use std::io::Write;
use std::fs::File;

pub const DEFAULT_CONTENT: &'static str = "
## Funzzy events file
# more details see: https://github.com/cristianoliveira/funzzy
#
# list here all the events and the commands that it should execute

- name: run my tests
  when:
    change: 'src/**'
    run: ls -a
";


/// # `InitCommand`
/// Creates a funzzy yaml boilerplate.
///
pub struct InitCommand {
    pub file_name: &'static str,
}
impl InitCommand {
    pub fn new() -> Self {
        InitCommand { file_name: "" }
    }
}

impl Command for InitCommand {
    fn execute(&self) -> Result<(), &str> {
        let mut yaml: File = match File::create(self.file_name) {
            Ok(f) => f,
            Err(err) => panic!("File wasn't created. Cause: {}", err),
        };

        if let Err(err) = yaml.write_all(DEFAULT_CONTENT.as_ref()) {
            panic!("Cannot write. Cause: {}", err)
        }

        Ok(())
    }
}
