extern crate notify;
extern crate yaml_rust;
extern crate glob;

use std::process::Command as ShellCommand;
use std::sync::mpsc::channel;
use std::error::Error;

use self::notify::{RecommendedWatcher, Watcher};
use self::yaml_rust::{Yaml, YamlLoader};

use cli::Command;
use rules;
use yaml;


pub const FILENAME: &'static str = ".watch.yaml";

/// # `WatchCommand`
///
/// Starts watcher to listen the change events configured
/// in watch.yaml
///
pub struct WatchCommand {
    watches: Watches,
    verbose: bool,
}

impl WatchCommand {
    pub fn new(watches: Watches, verbose: bool) -> Self {
        WatchCommand { watches: watches , verbose: verbose }
    }
}

impl Command for WatchCommand {
    fn execute(&self) -> Result<(), String> {

        let (tx, rx) = channel();
        let mut watcher: RecommendedWatcher = match Watcher::new(tx) {
            Ok(w) => w,
            Err(err) => panic!("Error while trying watch. Cause: {:?}", err),
        };

        if let Err(err) = watcher.watch(".") {
            panic!("Unable to watch current directory. Cause: {:?}", err)
        }

        println!("Watching.");
        while let Ok(event) = rx.recv() {
            let path: &str = if let Some(ref path_buf) = event.path {
                path_buf.to_str().expect("Error while cast path buffer.")
            } else {
                ""
            };

            if let Some(shell_commands) = self.watches.watch(path) {

                if self.verbose { println!("path: {}", path) };

                clear_shell();
                for mut cmd in shell_commands {
                    if self.verbose { println!("command: {:?}", cmd) };
                    if let Err(error) = cmd.status() {
                        return Err(String::from(error.description()));
                    }
                }
            }
        }
        Ok(())
    }
}


/// # Watches
///
/// Represents all items in the yaml config loaded.
///
pub struct Watches {
    items: Vec<rules::Rules>,
}
impl Watches {
    pub fn from_args(command: String) -> Self {
        let template = format!("
        - name: from command
          run: {command}
          change: '{path}'
        ",
         path = "**",
         command = command);

        Watches::load_from_str(&template)
    }

    pub fn from(plain_text: &str) -> Self {
        Watches::load_from_str(plain_text)
    }

    /// Returns the first watch found for the given path
    ///
    pub fn watch(&self, path: &str) -> Option<Vec<ShellCommand>> {
        print!("items {:?}", self.items);
        for rule in self.items.iter()
            .filter(|r| !r.ignore(path) && r.watch(path)) {
            return Some(rule.to_command());
        };
        None
    }

    fn load_from_str(plain_text: &str) -> Self {
        Watches { items: rules::from_yaml(plain_text) }
    }
}

fn clear_shell() {
    let _ = ShellCommand::new("clear").status();
}

#[cfg(test)]
mod tests {
    extern crate notify;
    extern crate yaml_rust;
    extern crate glob;

    use super::*;
    use std::process::Command as ShellCommand;
    use self::yaml_rust::YamlLoader;

    #[test]
    fn it_loads_from_args() {
        let args = String::from("cargo build");
        let watches = Watches::from_args(args);

        assert!(watches.watch("src/main.rs").is_some());
        assert!(watches.watch("test/main.rs").is_some());
        assert!(watches.watch(".").is_some());

        let result = watches.watch(".").unwrap();
        let mut expected = ShellCommand::new("cargo");
        expected.arg("build");
        assert_eq!(format!("{:?}", vec![expected]), format!("{:?}", result));
    }

    #[test]
    fn it_watches_test_path() {
        let file_content = "
        - name: my tests
          run: 'cargo tests'
          change: 'tests/**'
        ";
        let watches = Watches::from(file_content);
        assert!(watches.watch("/Users/crosa/others/funzzy/tests/test.rs").is_some());
        assert!(watches.watch("tests/tests.rs").is_some());
        assert!(watches.watch("tests/ruby.rb").is_some());
        assert!(watches.watch("tests/folder/other.rs").is_some())
    }

    #[test]
    fn it_doesnot_watch_test_path() {
        let file_content = "
        - name: my source
          run: 'cargo build'
          change: 'src/**'
        ";
        let watches = Watches::from(file_content);

        assert!(watches.watch("/Users/crosa/others/funzzy/events.yaml").is_none());
        assert!(watches.watch("tests/").is_none());
        assert!(watches.watch("tests/test.rs").is_none());
        assert!(watches.watch("tests/folder/other.rs").is_none());
    }

    #[test]
    fn it_creates_a_list_of_shell_commands() {
        let file_content = "
        - name: my source
          run: 'cargo build'
          change: 'src/**'
        ";
        let watches = Watches::from(file_content);
        let result = watches.watch("src/test.rs").unwrap();
        let mut expected = ShellCommand::new("cargo");
        expected.arg("build");
        assert_eq!(format!("{:?}", vec![expected]), format!("{:?}", result))
    }

    #[test]
    fn it_works_with_more_than_one_command() {
        let file_content = "
        - name: my source
          run: ['cargo build', 'cargo test']
          change: 'src/**'
        ";
        let watches = Watches::from(file_content);
        let result = watches.watch("src/test.rs").unwrap();

        let mut expected: Vec<ShellCommand> = vec![];
        let mut cmd = ShellCommand::new("cargo");
        cmd.arg("build");
        expected.push(cmd);

        let mut cmd2 = ShellCommand::new("cargo");
        cmd2.arg("test");
        expected.push(cmd2);

        assert_eq!(format!("{:?}", expected), format!("{:?}", result))
    }

    #[test]
    fn it_works_with_multiples_items() {
        let file_content = "
        - name: my source
          run: 'cargo build'
          change: 'src/**'

        - name: other
          run: 'cargo test'
          change: 'test/**'
        ";
        let watches = Watches::from(file_content);

        let result = watches.watch("test/test.rs").unwrap();
        let mut expected = ShellCommand::new("cargo");
        expected.arg("test");
        assert_eq!(format!("{:?}", expected), format!("{:?}", result[0]));

        let result_src = watches.watch("src/test.rs").unwrap();
        let mut expected_src = ShellCommand::new("cargo");
        expected_src.arg("build");
        assert_eq!(format!("{:?}", expected_src),
                   format!("{:?}", result_src[0]))
    }

    #[test]
    fn it_ignores_pattern() {
        let file_content = "
        - name: my source
          run: 'cargo build'
          change: 'src/**'
          ignore: 'src/test/**'
        ";
        let watches = Watches::from(file_content);
        assert!(watches.watch("src/other.rb").is_some());
        assert!(watches.watch("src/test.txt").is_some());
        assert!(watches.watch("src/test/other.tmp").is_none())
    }

    #[test]
    fn it_ignores_a_list_of_patterns() {
        let file_content = "
        - name: my source
          run: 'cargo build'
          change: 'src/**'
          ignore: ['src/test/**', 'src/tmp/**']
        ";
        let watches = Watches::from(file_content);
        assert!(watches.watch("src/other.rb").is_some());
        assert!(watches.watch("src/test.txt").is_some());
        assert!(watches.watch("src/tmp/test.txt").is_none());
        assert!(watches.watch("src/test/other.tmp").is_none())
    }
}
