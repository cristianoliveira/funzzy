extern crate notify;
extern crate git2;
extern crate yaml_rust;
extern crate glob;

use std::process::Command as ShellCommand;
use std::sync::mpsc::channel;
use self::glob::Pattern;

use self::notify::{RecommendedWatcher, Watcher};
use self::yaml_rust::{Yaml, YamlLoader};

use cli::Command;

/// # WatchCommand
///
/// Starts watcher to listen the change events configured
/// in watch.yaml
///
pub struct WatchCommand {
    watches: Watches,
}

impl WatchCommand {
    pub fn new(file_content: &str) -> Self {
        WatchCommand {
            watches: Watches::from(file_content)
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
            let watch = match event.path {
               Some(path_buf) => {
                   let path = path_buf.to_str().unwrap();
                   println!("path {:?}", path);

                   match self.watches.watch(&path) {
                       Some(mut cmd) => {
                           println!("comand: {:?}", cmd);
                           match cmd.status() {
                               Ok(_) => println!("executed"),
                               Err(err) => println!("Error {:?}", err),
                           };
                       },
                       None => println!("No command for this watch.")
                   }
               },
               None => println!("No path event.")
            };
        };
        Ok(())
    }


    fn help(&self) -> &str {
        " Watch command.
It starts to watch folders and execute the commands
that are located in events.yaml.

Example:
   # yaml file
   - name: says hello
     when:
       change: 'myfile.txt'
       do: 'echo \"Hello\"'
"
    }
}


/// # Watches
///
/// Represent all items in the yaml config loaded.
///
struct Watches {
    items: Vec<Yaml>,
}
impl Watches {
    pub fn from(plain_text: &str) -> Self{
        Watches {
            items: YamlLoader::load_from_str(&plain_text).unwrap(),
        }
    }

    /// Returns the first watch found for the given path
    /// it may return None if there is no item that match.
    ///
    pub fn watch(&self, path: &str) -> Option<ShellCommand>{
        for w in &self.items {
            let watched_path = w[0]["when"]["change"].as_str().unwrap();
            let watched_command = w[0]["when"]["run"].as_str().unwrap();
            println!("{:?} {:?}", watched_path, path);

            if Pattern::new(watched_path).unwrap().matches(path){
                let mut args: Vec<&str>= watched_command.split(' ').collect();
                let cmd = args.remove(0);
                println!("command {} args: {:?}", cmd, args);
                let mut shell = ShellCommand::new(cmd);
                shell.args(&args);
                return Some(shell)
            }
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
fn it_watches_test_path() {
    let file_content = "
- name: my tests
  when:
    change: '**/tests/**'
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
    change: '**/src/**'
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
    change: '**/src/**'
    run: 'cargo build'
";
    let watches = Watches::from(file_content);
    let result = watches.watch("src/test.rs").unwrap();
    let mut expected = ShellCommand::new("cargo");
    expected.arg("build");
    assert_eq!(format!("{:?}", expected),  format!("{:?}", result))
}
