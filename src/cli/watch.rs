use cli::errors::CliError;
use cli::Command;


/// # InitCommand
/// Creates a funzzy yaml boilerplate.
/// 
pub struct WatchCommand;
impl Command for WatchCommand {
    fn execute(&self) -> Result<(), CliError>{
        println!("Watching.");
        Ok(())
    }

    fn help(&self) -> &str {
        " Watch command.
It starts to watch folders and execute the commands
that are located in events.yaml.

Example:
   # yaml file
   - name: says hello
     when: 'myfile.txt'
     do: 'echo \"Hello\"'
"
    }
}
