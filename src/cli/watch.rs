extern crate notify;
extern crate glob;

use std::process::Command as ShellCommand;
use std::sync::mpsc::channel;

use self::notify::{RecommendedWatcher, Watcher};

use cli::Command;
use rules;
use cmd;

pub const DEFAULT_FILENAME: &'static str = ".watch.yaml";

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
        if verbose { println!("watches {:?}", watches); }
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
                for command in shell_commands {
                    if self.verbose { println!("command: {:?}", command) };
                    try!(cmd::execute(command))
                }
            }
        }
        Ok(())
    }
}


/// # Watches
///
/// Represents all rules in the yaml config loaded.
///
#[derive(Debug)]
pub struct Watches {
    rules: Vec<rules::Rules>,
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

    pub fn new(rules: Vec<rules::Rules>) -> Self {
        Watches { rules: rules }
    }

    fn load_from_str(plain_text: &str) -> Self {
        Watches { rules: rules::from_yaml(plain_text) }
    }


    /// Returns the first rule found for the given path
    ///
    pub fn watch(&self, path: &str) -> Option<Vec<String>> {
        for rule in self.rules.iter()
            .filter(|r| !r.ignore(path) && r.watch(path)) {
            return Some(rule.commands());
        };
        None
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

    #[test]
    fn it_loads_from_args() {
        let args = String::from("cargo build");
        let watches = Watches::from_args(args);

        assert!(watches.watch("src/main.rs").is_some());
        assert!(watches.watch("test/main.rs").is_some());
        assert!(watches.watch(".").is_some());

        let result = watches.watch(".").unwrap();
        assert_eq!(vec!["cargo build"], result);
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
    fn it_watches_specific_path() {
        let file_content = "
        - name: my tests
          run: 'cargo tests'
          change: './tests/foo/bar.rs'
        ";
        let watches = Watches::from(file_content);
        assert!(watches.watch("./tests/foo/bar.rs").is_some())
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
        assert_eq!(vec!["cargo build"], result)
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

        assert_eq!(vec!["cargo build", "cargo test"], result)
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
        assert_eq!(vec!["cargo test"], result);

        let result_src = watches.watch("src/test.rs").unwrap();
        assert_eq!(vec!["cargo build"], result_src);
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
