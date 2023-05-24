extern crate notify;

use std::process::Command as ShellCommand;
use std::sync::mpsc::channel;

use self::notify::{DebouncedEvent, RecommendedWatcher, RecursiveMode, Watcher};
use std::time::Duration;

use cli::Command;
use cmd;
use rules;

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
        if verbose {
            println!("watches {:?}", watches);
        }
        WatchCommand {
            watches: watches,
            verbose: verbose,
        }
    }

    fn run(&self, commands: &Vec<String>) -> Result<(), String> {
        for command in commands {
            if self.verbose {
                println!("command: {:?}", command)
            };
            println!(" ----- funzzy running: {} -------", command);
            cmd::execute(String::from(command))?
        }
        Ok(())
    }

    fn run_rules(&self, rules: Vec<Vec<String>>) -> Result<(), String> {
        clear_shell();
        let results = rules
            .iter()
            .map(|rule_cmds| self.run(&rule_cmds))
            .find(|r| match r {
                Ok(_) => false,
                Err(_) => true,
            });

        match results {
            Some(Err(err)) => Err(err),
            _ => Ok(()),
        }
    }
}

impl Command for WatchCommand {
    fn execute(&self) -> Result<(), String> {
        if self.verbose {
            println!("---- Verbose mode enabled. ----")
        };

        let (tx, rx) = channel();
        let mut watcher: RecommendedWatcher = match Watcher::new(tx, Duration::from_secs(2)) {
            Ok(w) => w,
            Err(err) => panic!("Error while trying watch. Cause: {:?}", err),
        };

        if let Err(err) = watcher.watch(".", RecursiveMode::Recursive) {
            panic!("Unable to watch current directory. Cause: {:?}", err)
        }

        if let Some(rules) = self.watches.run_on_init() {
            println!("Running on init commands.");

            self.run_rules(rules)?
        }

        println!("Watching...");
        while let Ok(event) = rx.recv() {
            if let DebouncedEvent::Create(path) = event {
                let path_str = path.into_os_string().into_string().unwrap();
                if let Some(rules) = self.watches.watch(&*path_str) {
                    if self.verbose {
                        println!("Triggered by change in: {}", path_str)
                    };

                    self.run_rules(rules)?
                }
            }
        }
        Ok(())
    }
}

// Run the shell commans

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
        let template = format!(
            "
        - name: from command
          run: {command}
          change: '{path}'
        ",
            path = "**",
            command = command
        );

        Watches::load_from_str(&template)
    }

    pub fn from(plain_text: &str) -> Self {
        Watches::load_from_str(plain_text)
    }

    pub fn new(rules: Vec<rules::Rules>) -> Self {
        Watches { rules: rules }
    }

    pub fn filter(&self, predicate: impl Fn(&rules::Rules) -> bool) -> Self {
        let targeted = self
            .rules
            .iter()
            .filter(|x| predicate(*x))
            .map(|x| x.clone())
            .collect::<Vec<rules::Rules>>();

        if targeted.len() == 0 {
            panic!("No rules found for the given filter")
        }

        Watches { rules: targeted }
    }

    fn load_from_str(plain_text: &str) -> Self {
        Watches {
            rules: rules::from_yaml(plain_text),
        }
    }

    /// Returns the commands for first rule found for the given path
    ///
    pub fn watch(&self, path: &str) -> Option<Vec<Vec<String>>> {
        let cmds = self
            .rules
            .iter()
            .filter(|r| !r.ignore(path) && r.watch(path))
            .map(|r| r.commands())
            .collect::<Vec<Vec<String>>>();

        match cmds.len() {
            0 => None,
            _ => Some(cmds),
        }
    }

    /// Returns the commands for the rules that should run on init
    ///
    pub fn run_on_init(&self) -> Option<Vec<Vec<String>>> {
        let cmds = self
            .rules
            .iter()
            .filter(|r| r.run_on_init())
            .map(|r| r.commands())
            .collect::<Vec<Vec<String>>>();

        match cmds.len() {
            0 => None,
            _ => Some(cmds),
        }
    }
}

fn clear_shell() {
    let _ = ShellCommand::new("clear").status();
}

#[cfg(test)]
mod tests {
    extern crate glob;
    extern crate notify;
    extern crate yaml_rust;

    use super::*;

    #[test]
    fn it_loads_from_args() {
        let args = String::from("cargo build");
        let watches = Watches::from_args(args);

        assert!(watches.watch("src/main.rs").is_some());
        assert!(watches.watch("test/main.rs").is_some());
        assert!(watches.watch(".").is_some());

        let result = watches.watch(".").unwrap();
        assert_eq!(vec!["cargo build"], result[0]);
    }

    #[test]
    fn it_watches_test_path() {
        let file_content = "
        - name: my tests
          run: 'cargo tests'
          change: 'tests/**'
        ";
        let watches = Watches::from(file_content);
        assert!(watches
            .watch("/Users/crosa/others/funzzy/tests/test.rs")
            .is_some());
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

        assert!(watches
            .watch("/Users/crosa/others/funzzy/events.yaml")
            .is_none());
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
        assert_eq!(vec!["cargo build"], result[0])
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
        assert_eq!(vec!["cargo test"], result[0]);

        let result_src = watches.watch("src/test.rs").unwrap();
        assert_eq!(vec!["cargo build"], result_src[0]);
    }

    #[test]
    fn it_allows_many_rules_watching_same_path() {
        let file_content = "
        - name: same path
          run: 'echo same'
          change: '**'

        - name: my source
          run: 'cargo build'
          change: 'src/**'

        - name: other
          run: 'cargo test'
          change: 'test/**'
        ";
        let watches = Watches::from(file_content);

        let result = watches.watch("test/test.rs").unwrap();
        assert_eq!(vec!["echo same"], result[0]);
        assert_eq!(vec!["cargo test"], result[1]);

        let result_multiple = watches.watch("src/test.rs").unwrap();
        assert_eq!(vec!["echo same"], result_multiple[0]);
        assert_eq!(vec!["cargo build"], result_multiple[1]);
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

    #[test]
    fn it_returns_on_init_rules() {
        let file_content = "
            - name: my source
              run: 'cargo build'
              change: 'src/**'
              run_on_init: true

            - name: my source
              run: ['cat foo', 'cat bar']
              change: 'src/**'
              run_on_init: true

            - name: other
              run: 'cargo test'
              change: 'test/**'
            ";
        let watches = Watches::from(file_content);
        let results = watches.run_on_init().unwrap();

        assert_eq!(results[0], vec!["cargo build".to_string(),]);
        assert_eq!(
            results[1],
            vec!["cat foo".to_string(), "cat bar".to_string(),]
        );
    }
}
