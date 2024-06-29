use std::io::prelude::*;
use std::{
    env,
    fs::File,
    process::{Command, Stdio},
    thread::sleep,
    time::Duration,
};

struct ScopeCall<F: FnMut()> {
    c: F,
}
impl<F: FnMut()> Drop for ScopeCall<F> {
    fn drop(&mut self) {
        println!("Cleaning up...");
        (self.c)();
    }
}

macro_rules! defer {
    ($e:expr) => {
        let _scope_call = ScopeCall {
            c: || -> () {
                $e;
            },
        };
    };
}

macro_rules! wait_until {
    ($e:expr) => {
        for _ in 0..100 {
            let result = $e;
            if result {
                break;
            }
            sleep(Duration::from_millis(100));
        }
    };
}

#[test]
fn test_it_watch_a_simple_case() {
    let test_log = "test_it_watch_a_simple_case.log";
    let clear_char = "[H[J";

    let dir = env::current_dir().expect("failed to get current directory");
    let bin_path = dir.join("target/debug/fzz");

    let _ = std::fs::remove_file(dir.join(test_log));
    let output_log = File::create(dir.join(test_log)).expect("error log file");
    output_log.set_len(0).expect("failed to truncate file");
    let stdio = Stdio::from(output_log);

    let mut child = Command::new(bin_path)
        .arg("-c")
        .arg(dir.join("examples/simple-case.yml"))
        .stdout(stdio)
        .spawn()
        .expect("failed to spawn child");

    defer!({
        child.kill().expect("failed to kill child");
        let _ = std::fs::remove_file(dir.join(test_log));
    });

    let mut output = String::new();
    let mut log = File::open(dir.join(test_log)).expect("failed to open file");

    // wait for the watcher to start
    wait_until!({
        sleep(Duration::from_millis(100));
        log.read_to_string(&mut output)
            .expect("failed to read from file");

        output.contains("Watching...")
    });

    output.truncate(0);

    let mut file = File::create(dir.join("examples/workdir/trigger-watcher.txt"))
        .expect("failed to open file");
    file.write_all(b"foo\n").expect("failed to write to file");

    wait_until!({
        log.read_to_string(&mut output)
            .expect("failed to read from file");

        output.contains("Funzzy: results")
    });

    assert_eq!(
        output.replace(clear_char, ""),
        "
Funzzy: clear 


Funzzy: echo first 

first

Funzzy: echo second 

second

Funzzy: echo complex | sed s/complex/third/g 

third
Funzzy results ----------------------------
All tasks finished successfully.
"
    );
}
