//! Regression proof for terminal job-control safety in the watch shortcut.

#![cfg(all(feature = "test-integration", unix))]

use nix::pty::openpty;
use std::fs::File;
use std::io::Read;
use std::os::unix::process::CommandExt;
use std::process::{Child, Command, Stdio};
use std::time::{Duration, Instant};

const HELPER_ENV: &str = "FUNZZY_BACKGROUND_TTY_HELPER";

struct Scratch(std::path::PathBuf);

impl Drop for Scratch {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.0);
    }
}

fn terminate(child: &mut Child) {
    let pid = child.id() as nix::libc::pid_t;
    unsafe {
        let _ = nix::libc::kill(pid, nix::libc::SIGKILL);
    }
    let _ = child.wait();
}

#[test]
fn background_tty_process_group_does_not_stop_watcher() {
    let root = std::env::temp_dir().join(format!("funzzy-background-tty-{}", std::process::id()));
    let _scratch = Scratch(root.clone());
    let _ = std::fs::remove_dir_all(&root);
    std::fs::create_dir_all(root.join("src")).expect("create scratch watch root");
    let socket = root.join("control.sock");
    let config = root.join(".watch.yaml");
    std::fs::write(
        &config,
        format!(
            "on:\n  socket: {:?}\n  watch_backend: poll\n  poll_interval: 20ms\njobs:\n  - name: idle\n    run: 'true'\n    change: 'src/**'\n    run_on_init: false\n",
            socket.display().to_string()
        ),
    )
    .expect("write watcher config");

    let pty = openpty(None, None).expect("open controlling pty");
    let _master = File::from(pty.master);
    let slave = File::from(pty.slave);
    let mut helper = Command::new(std::env::current_exe().expect("current test executable"));
    helper
        .args([
            "--exact",
            "background_tty_controller_helper",
            "--ignored",
            "--nocapture",
            "--test-threads=1",
        ])
        .env(HELPER_ENV, "1")
        .env("FUNZZY_BACKGROUND_TTY_ROOT", &root)
        .env("FUNZZY_BACKGROUND_TTY_CONFIG", &config)
        .env("FUNZZY_BACKGROUND_TTY_SOCKET", &socket)
        .stdin(Stdio::from(slave))
        .stdout(Stdio::piped())
        .stderr(Stdio::piped());

    unsafe {
        helper.pre_exec(|| {
            if nix::libc::setsid() == -1 {
                return Err(std::io::Error::last_os_error());
            }
            if nix::libc::ioctl(0, nix::libc::TIOCSCTTY as _, 0) == -1 {
                return Err(std::io::Error::last_os_error());
            }
            if nix::libc::tcsetpgrp(0, nix::libc::getpgrp()) == -1 {
                return Err(std::io::Error::last_os_error());
            }
            Ok(())
        });
    }

    let mut helper = helper.spawn().expect("spawn controlling-terminal helper");
    let deadline = Instant::now() + Duration::from_secs(10);
    let status = loop {
        if let Some(status) = helper.try_wait().expect("poll helper") {
            break status;
        }
        if Instant::now() >= deadline {
            terminate(&mut helper);
            panic!("controlling-terminal helper timed out");
        }
        std::thread::sleep(Duration::from_millis(10));
    };
    let mut stdout = String::new();
    let mut stderr = String::new();
    helper
        .stdout
        .take()
        .expect("helper stdout")
        .read_to_string(&mut stdout)
        .expect("read helper stdout");
    helper
        .stderr
        .take()
        .expect("helper stderr")
        .read_to_string(&mut stderr)
        .expect("read helper stderr");
    assert!(
        status.success(),
        "background-terminal helper failed: {status}\nstdout:\n{stdout}\nstderr:\n{stderr}"
    );
}

#[test]
#[ignore = "run only as the controlling-terminal subprocess"]
fn background_tty_controller_helper() {
    if std::env::var_os(HELPER_ENV).is_none() {
        return;
    }
    let root = std::env::var_os("FUNZZY_BACKGROUND_TTY_ROOT").expect("helper root");
    let config = std::env::var_os("FUNZZY_BACKGROUND_TTY_CONFIG").expect("helper config");
    let socket = std::path::PathBuf::from(
        std::env::var_os("FUNZZY_BACKGROUND_TTY_SOCKET").expect("helper socket"),
    );
    let mut watcher = Command::new(env!("CARGO_BIN_EXE_fzz"));
    watcher
        .current_dir(root)
        .arg("-c")
        .arg(config)
        .env("_TEST_FUNZZY_COLORED", "0")
        .env("_TEST_FUNZZY_BAIL", "0")
        .stdin(Stdio::inherit())
        .stdout(Stdio::null())
        .stderr(Stdio::null());
    unsafe {
        watcher.pre_exec(|| {
            let _ = nix::libc::signal(nix::libc::SIGTTIN, nix::libc::SIG_DFL);
            let _ = nix::libc::signal(nix::libc::SIGTTOU, nix::libc::SIG_DFL);
            if nix::libc::setpgid(0, 0) == -1 {
                return Err(std::io::Error::last_os_error());
            }
            Ok(())
        });
    }
    let mut watcher = watcher.spawn().expect("spawn background watcher");
    let pid = watcher.id() as nix::libc::pid_t;
    let deadline = Instant::now() + Duration::from_secs(5);
    loop {
        let mut status = 0;
        let observed = unsafe {
            nix::libc::waitpid(pid, &mut status, nix::libc::WNOHANG | nix::libc::WUNTRACED)
        };
        if observed == pid {
            if nix::libc::WIFSTOPPED(status) {
                let signal = nix::libc::WSTOPSIG(status);
                terminate(&mut watcher);
                panic!("background watcher stopped by job-control signal {signal}");
            }
            if nix::libc::WIFEXITED(status) {
                panic!(
                    "background watcher exited early with code {}",
                    nix::libc::WEXITSTATUS(status)
                );
            }
            if nix::libc::WIFSIGNALED(status) {
                panic!(
                    "background watcher terminated early by signal {}",
                    nix::libc::WTERMSIG(status)
                );
            }
        } else if observed == -1 {
            panic!("waitpid failed: {}", std::io::Error::last_os_error());
        }

        if std::os::unix::net::UnixStream::connect(&socket).is_ok() {
            terminate(&mut watcher);
            return;
        }
        if Instant::now() >= deadline {
            terminate(&mut watcher);
            panic!("background watcher never became ready");
        }
        std::thread::sleep(Duration::from_millis(10));
    }
}
