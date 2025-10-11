use crate::logging;
use crate::stdout;
use std::io::{BufRead, BufReader, Write};
use std::process::{Child, Command, Stdio};
use std::thread;

pub fn execute(command: &String) -> Result<(), String> {
    println!();
    logging::log_line("");
    stdout::info(&format!("{} \n", String::from(command)));

    let mut cmd = prepare_command(command);

    if logging::is_enabled() {
        cmd.stdout(Stdio::piped());
        cmd.stderr(Stdio::piped());

        let mut child = cmd
            .spawn()
            .map_err(|error| format!("Command {} has errored with {}", command, error))?;

        forward_child_output(&mut child);

        match child.wait() {
            Ok(status) if status.success() => Ok(()),
            Ok(status) => Err(format!("Command {} has failed with {}", command, status)),
            Err(error) => Err(format!("Command {} has errored with {}", command, error)),
        }
    } else {
        match cmd.status() {
            Err(error) => Err(format!("Command {} has errored with {}", command, error)),
            Ok(status) => {
                if status.success() {
                    Ok(())
                } else {
                    Err(format!("Command {} has failed with {}", command, status))
                }
            }
        }
    }
}

pub fn spawn(command: &String) -> Result<Child, String> {
    println!();
    logging::log_line("");
    stdout::info(&format!("{} \n", String::from(command)));

    let mut cmd = prepare_command(command);

    if logging::is_enabled() {
        cmd.stdout(Stdio::piped());
        cmd.stderr(Stdio::piped());

        match cmd.spawn() {
            Ok(mut child) => {
                forward_child_output(&mut child);
                Ok(child)
            }
            Err(error) => Err(format!("Command {} has errored with {}", command, error)),
        }
    } else {
        match cmd.spawn() {
            Ok(child) => Ok(child),
            Err(error) => Err(format!("Command {} has errored with {}", command, error)),
        }
    }
}

#[test]
fn it_spawn_a_command_returning_a_child_ref() {
    let result = match spawn(&String::from("echo 'foo'")) {
        Ok(mut child) => child.wait().expect("fail to wait"),
        Err(err) => panic!("{:?}", err),
    };

    assert_eq!(format!("{}", result), "exit status: 0")
}

#[test]
fn it_executes_a_command() {
    let result = match execute(&String::from("echo 'foo'")) {
        Ok(_) => true,
        Err(err) => panic!("{:?}", err),
    };

    assert!(result)
}

fn prepare_command(command: &String) -> Command {
    let shell = std::env::var("SHELL").unwrap_or(String::from("/bin/sh"));
    let mut cmd = Command::new(shell);
    cmd.arg("-c").arg(command);
    cmd
}

fn forward_child_output(child: &mut Child) {
    if let Some(stdout) = child.stdout.take() {
        spawn_forwarding_thread(stdout, false);
    }

    if let Some(stderr) = child.stderr.take() {
        spawn_forwarding_thread(stderr, true);
    }
}

fn spawn_forwarding_thread<R: std::io::Read + Send + 'static>(reader: R, is_stderr: bool) {
    thread::spawn(move || {
        let mut reader = BufReader::new(reader);
        let mut line = String::new();

        loop {
            line.clear();

            match reader.read_line(&mut line) {
                Ok(0) => break,
                Ok(_) => {
                    if is_stderr {
                        eprint!("{}", line);
                        let _ = std::io::stderr().flush();
                    } else {
                        print!("{}", line);
                        let _ = std::io::stdout().flush();
                    }

                    logging::log_plain(&line);
                }
                Err(_) => break,
            }
        }
    });
}
