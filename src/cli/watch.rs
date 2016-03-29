extern crate notify;
extern crate yaml_rust;
extern crate glob;

use std::process::Command as ShellCommand;
use std::sync::mpsc::channel;
use self::glob::Pattern;

use self::notify::{RecommendedWatcher, Watcher};
use self::yaml_rust::{Yaml, YamlLoader};

use cli::Command;


pub const FILENAME: &'static str = ".watch.yaml";

/// # `WatchCommand`
///
/// Starts watcher to listen the change events configured
/// in watch.yaml
///
pub struct WatchCommand {
    watches: Watches,
}

impl WatchCommand {
    pub fn new(watches: Watches) -> Self {
        WatchCommand {
            watches: watches
        }
    }
}

impl Command for WatchCommand {

    fn execute(&self) -> Result<(), &str>{

        let (tx, rx) = channel();
        let mut watcher: RecommendedWatcher = match Watcher::new(tx) {
            Ok(w) => w,
            Err(err) => panic!("Error while trying watch. Cause: {:?}",err)
        };

        match watcher.watch(".") {
            Ok(()) => {},
            Err(_) => panic!("Unable to watch current directory"),
        }

        println!("Watching.");
        while let Ok(event) = rx.recv() {
            let path: &str = if let Some(ref path_buf) = event.path {
                path_buf.to_str().unwrap()
            } else {
                ""
            };

            if let Some(mut cmd) = self.watches.watch(&path) {
                match cmd.status() {
                    Ok(_) => println!("executed"),
                    Err(err) => println!("Error {:?}", err),
                }
            }
        };
        Ok(())
    }
}


/// # Watches
///
/// Represent all items in the yaml config loaded.
///
pub struct Watches {
    items: Vec<Yaml>,
}
impl Watches {
    pub fn from_args(args: Vec<String>) -> Self {
        let template =
        format!("
        - name: from command
          when:
            change: '{path}'
            run: {command}
        ", path="**", command=args[0]);

        Watches {
            items: YamlLoader::load_from_str(&template).unwrap(),
        }
    }

    pub fn from(plain_text: &str) -> Self {
        Watches {
            items: YamlLoader::load_from_str(&plain_text).unwrap(),
        }
    }

    /// Returns the first watch found for the given path
    /// it may return None if there is no item that match.
    ///
    pub fn watch(&self, path: &str) -> Option<ShellCommand>{
        match self.items[0] {
            Yaml::Array(ref items) => {
                for i in items {
                    let pattern = i["when"]["change"].as_str().unwrap();
                    let command = i["when"]["run"].as_str().unwrap();

                    if Pattern::new(&format!("**/{}", pattern)).unwrap().matches(path){
                        println!("Running: {}", i["name"].as_str().unwrap());
                        let mut args: Vec<&str>= command.split(' ').collect();
                        let cmd = args.remove(0);

                        let mut shell = ShellCommand::new(cmd);
                        shell.args(&args);

                        return Some(shell)
                    }
                };
            },
            _ => panic!("Yaml format unkown.")
        };
        None
    }
}


#[test]
fn it_loads_from_yaml_file() {
    let file_content = "
- name: my tests
  when:
    change: tests/*
    run: cargo tests
";
    let content = YamlLoader::load_from_str(&file_content).unwrap();
    let watches = Watches::from(file_content);
    assert_eq!(content[0], watches.items[0]);
    assert_eq!(content[0]["when"], watches.items[0]["when"]);
    assert_eq!(content[0]["when"]["change"],
               watches.items[0]["when"]["change"])
}

#[test]
fn it_loads_from_args() {
    let args = vec![String::from("cargo build")];
    let watches = Watches::from_args(args);

    println!("{:?}", watches.items[0]);
    assert!(watches.watch("src/main.rs").is_some());
    assert!(watches.watch("test/main.rs").is_some());
    assert!(watches.watch(".").is_some());

    let result = watches.watch(".").unwrap();
    let mut expected = ShellCommand::new("cargo");
    expected.arg("build");
    assert_eq!(format!("{:?}", expected),  format!("{:?}", result));
}

#[test]
fn it_watches_test_path() {
    let file_content = "
- name: my tests
  when:
    change: 'tests/**'
    run: 'cargo tests'
";
    let watches = Watches::from(file_content);
    assert!(watches.watch("/Users/crosa/others/funzzy/tests/test.rs").is_some());
    assert!(watches.watch("tests/tests.rs").is_some());
    assert!(watches.watch("tests/ruby.rb").is_some());
    assert!(watches.watch("tests/folder/other.rs").is_some())
}

#[test]
fn it_doesnot_watches_test_path() {
    let file_content = "
- name: my source
  when:
    change: 'src/**'
    run: 'cargo build'
";
    let watches = Watches::from(file_content);

    assert!(watches.watch("/Users/crosa/others/funzzy/events.yaml").is_none());
    assert!(watches.watch("tests/").is_none());
    assert!(watches.watch("tests/test.rs").is_none());
    assert!(watches.watch("tests/folder/other.rs").is_none());
}

#[test]
fn it_creates_shell_command() {
    let file_content = "
- name: my source
  when:
    change: 'src/**'
    run: 'cargo build'
";
    let watches = Watches::from(file_content);
    let result = watches.watch("src/test.rs").unwrap();
    let mut expected = ShellCommand::new("cargo");
    expected.arg("build");
    assert_eq!(format!("{:?}", expected),  format!("{:?}", result))
}

#[test]
fn it_works_with_multiples_itens() {
    let file_content = "
- name: my source
  when:
    change: 'src/**'
    run: 'cargo build'

- name: other
  when:
    change: 'test/**'
    run: 'cargo test'
";
    let watches = Watches::from(file_content);

    let result = watches.watch("test/test.rs").unwrap();
    let mut expected = ShellCommand::new("cargo");
    expected.arg("test");
    assert_eq!(format!("{:?}", expected),  format!("{:?}", result));

    let result_src = watches.watch("src/test.rs").unwrap();
    let mut expected_src = ShellCommand::new("cargo");
    expected_src.arg("build");
    assert_eq!(format!("{:?}", expected_src),  format!("{:?}", result_src))
}
