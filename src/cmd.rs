use crate::logging;
use crate::stdout;
use std::io::{BufRead, BufReader, Write};
use std::process::{Child, Command, ExitStatus, Stdio};
use std::thread;

/// Keeps track of the threads that forward the child's stdout/stderr so they
/// can be joined once the child process exits.
struct ForwardHandles {
    stdout: Option<thread::JoinHandle<()>>,
    stderr: Option<thread::JoinHandle<()>>,
}

impl ForwardHandles {
    fn new() -> Self {
        Self {
            stdout: None,
            stderr: None,
        }
    }

    fn join(&mut self) {
        if let Some(handle) = self.stdout.take() {
            let _ = handle.join();
        }

        if let Some(handle) = self.stderr.take() {
            let _ = handle.join();
        }
    }

    fn discard(&mut self) {
        self.stdout.take();
        self.stderr.take();
    }
}

/// Wraps a [`Child`] whose stdout/stderr are being forwarded by background
/// threads. The wrapper makes sure those threads are joined once the child
/// finishes so no output is lost and the process shuts down cleanly.
pub struct LoggedChild {
    child: Child,
    forward_handles: ForwardHandles,
    has_finished: bool,
}

impl LoggedChild {
    fn new(mut child: Child) -> Self {
        let forward_handles = forward_child_output(&mut child);
        Self {
            child,
            forward_handles,
            has_finished: false,
        }
    }

    pub fn id(&self) -> u32 {
        self.child.id()
    }

    pub fn try_wait(&mut self) -> std::io::Result<Option<ExitStatus>> {
        let result = self.child.try_wait()?;

        if result.is_some() {
            self.join_forwarding_threads();
        }

        Ok(result)
    }

    pub fn wait(&mut self) -> std::io::Result<ExitStatus> {
        let status = self.child.wait();

        match status {
            Ok(ref _status) => {
                self.join_forwarding_threads();
            }
            Err(_) => {
                self.forward_handles.discard();
            }
        }

        status
    }

    pub fn kill(&mut self) -> std::io::Result<()> {
        self.child.kill()
    }

    fn join_forwarding_threads(&mut self) {
        if !self.has_finished {
            self.forward_handles.join();
            self.has_finished = true;
        }
    }
}

impl Drop for LoggedChild {
    fn drop(&mut self) {
        if !self.has_finished {
            if let Ok(Some(_)) = self.child.try_wait() {
                self.forward_handles.join();
            } else {
                self.forward_handles.discard();
            }
        }
    }
}

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

        let mut handles = forward_child_output(&mut child);

        match child.wait() {
            Ok(status) if status.success() => {
                handles.join();
                Ok(())
            }
            Ok(status) => {
                handles.join();
                Err(format!("Command {} has failed with {}", command, status))
            }
            Err(error) => {
                handles.discard();
                Err(format!("Command {} has errored with {}", command, error))
            }
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

pub fn spawn(command: &String) -> Result<LoggedChild, String> {
    println!();
    logging::log_line("");
    stdout::info(&format!("{} \n", String::from(command)));

    let mut cmd = prepare_command(command);

    if logging::is_enabled() {
        cmd.stdout(Stdio::piped());
        cmd.stderr(Stdio::piped());

        match cmd.spawn() {
            Ok(child) => Ok(LoggedChild::new(child)),
            Err(error) => Err(format!("Command {} has errored with {}", command, error)),
        }
    } else {
        match cmd.spawn() {
            Ok(child) => Ok(LoggedChild::new(child)),
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

fn forward_child_output(child: &mut Child) -> ForwardHandles {
    let mut handles = ForwardHandles::new();

    if let Some(stdout) = child.stdout.take() {
        handles.stdout = Some(spawn_forwarding_thread(stdout, false));
    }

    if let Some(stderr) = child.stderr.take() {
        handles.stderr = Some(spawn_forwarding_thread(stderr, true));
    }

    handles
}

fn spawn_forwarding_thread<R: std::io::Read + Send + 'static>(
    reader: R,
    is_stderr: bool,
) -> thread::JoinHandle<()> {
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
    })
}
