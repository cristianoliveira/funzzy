extern crate notify;
extern crate git2;
extern crate yaml_rust;

use std::env;
use std::process::Command as SysCommand;
use std::sync::mpsc::channel;
use std::sync::{Arc, Mutex};
use self::notify::{RecommendedWatcher, Watcher};
use self::yaml_rust::{Yaml, YamlLoader, YamlEmitter};
use std::thread;

use cli::Command;

/// # WatchCommand
///
/// Starts watcher to listen the change envents configured
/// in watch.yaml
///
pub struct WatchCommand {
    watches: Watches,
}

impl WatchCommand {
    pub fn new() -> Self {
        WatchCommand {
            watches: Watches::from("")
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
            match event.path {
               Some(path_buf) => {
                   let path = path_buf.to_str().unwrap();
                   if self.watches.watch(&path) {
                       println!("{:?}", path)
                   }
               },
               None => println!("No path found.")
            }
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
/// Represent the yaml config loaded by watch command.
///
struct Watches {
    data: Vec<Yaml>,
}
impl Watches {
    pub fn from(plain_text: &str) -> Self{
        Watches {
            data: YamlLoader::load_from_str(&plain_text).unwrap(),
        }
    }

    pub fn watch(&self, path: &str) -> bool {
        1 == 1
    }
}
