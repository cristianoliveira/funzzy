#![cfg(all(feature = "test-integration", unix))]

use nix::pty::openpty;
use std::fs::File;
use std::io::{Read, Write};
use std::os::unix::fs::PermissionsExt;
use std::os::unix::process::CommandExt;
use std::process::{Child, Command, Stdio};
use std::time::{Duration, Instant};

struct Scratch(std::path::PathBuf);
impl Drop for Scratch {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.0);
    }
}

fn wait_for(master: &mut File, output: &mut String, needle: &str) {
    let deadline = Instant::now() + Duration::from_secs(15);
    let mut bytes = [0; 4096];
    while Instant::now() < deadline {
        let mut poll = nix::libc::pollfd {
            fd: std::os::fd::AsRawFd::as_raw_fd(master),
            events: nix::libc::POLLIN,
            revents: 0,
        };
        if unsafe { nix::libc::poll(&mut poll, 1, 100) } > 0 {
            match master.read(&mut bytes) {
                Ok(size) => output.push_str(&String::from_utf8_lossy(&bytes[..size])),
                Err(error) if error.kind() == std::io::ErrorKind::WouldBlock => {}
                Err(error) => panic!("read pty: {error}"),
            }
        }
        if output.contains(needle) {
            return;
        }
    }
    panic!("timed out waiting for {needle:?}; terminal output:\n{output}");
}

struct ChildGuard(Child);

impl Drop for ChildGuard {
    fn drop(&mut self) {
        let pid = self.0.id() as nix::libc::pid_t;
        unsafe {
            let _ = nix::libc::kill(-pid, nix::libc::SIGTERM);
        }
        unsafe {
            let _ = nix::libc::kill(pid, nix::libc::SIGKILL);
            let _ = nix::libc::kill(-pid, nix::libc::SIGKILL);
        }
        let deadline = Instant::now() + Duration::from_secs(2);
        while Instant::now() < deadline {
            if self.0.try_wait().ok().flatten().is_some() {
                return;
            }
            std::thread::sleep(Duration::from_millis(20));
        }
    }
}

#[test]
fn ctrl_y_copies_latest_failure_from_foreground_blocking_and_nonblocking_watchers() {
    for non_block in [false, true] {
        let root = std::env::temp_dir().join(format!(
            "funzzy-copy-failure-{}-{non_block}",
            std::process::id()
        ));
        let _scratch = Scratch(root.clone());
        let _ = std::fs::remove_dir_all(&root);
        std::fs::create_dir_all(root.join("src")).unwrap();
        std::fs::create_dir_all(root.join("bin")).unwrap();
        let copied = root.join("copied.txt");
        #[cfg(target_os = "macos")]
        let helper = "pbcopy";
        #[cfg(not(target_os = "macos"))]
        let helper = "wl-copy";
        let helper_path = root.join("bin").join(helper);
        std::fs::write(
            &helper_path,
            format!("#!/bin/sh\n/bin/cat > '{}'\n", copied.display()),
        )
        .unwrap();
        let mut permissions = std::fs::metadata(&helper_path).unwrap().permissions();
        permissions.set_mode(0o755);
        std::fs::set_permissions(&helper_path, permissions).unwrap();
        #[cfg(unix)]
        std::os::unix::fs::symlink("/bin/sh", root.join("bin/sh")).unwrap();

        let socket_line = if non_block {
            format!(
                "  socket: {:?}\n",
                root.join("watch.sock").display().to_string()
            )
        } else {
            String::new()
        };
        std::fs::write(
            root.join(".watch.yaml"),
            format!(
                "on:\n  change: '**/*'\n  watch_backend: poll\n  poll_interval: 20ms\n{socket_line}jobs:\n  - name: clipboard regression\n    run: \"/bin/sh -c 'if [ -f pass.marker ]; then echo pass-marker; exit 0; else echo stdout-marker; echo stderr-marker >&2; exit 1; fi'\"\n    change: '*.txt'\n"
            ),
        )
        .unwrap();

        let pty = openpty(None, None).unwrap();
        let mut master = File::from(pty.master);
        let flags = unsafe {
            nix::libc::fcntl(std::os::fd::AsRawFd::as_raw_fd(&master), nix::libc::F_GETFL)
        };
        assert!(flags >= 0);
        assert_eq!(
            unsafe {
                nix::libc::fcntl(
                    std::os::fd::AsRawFd::as_raw_fd(&master),
                    nix::libc::F_SETFL,
                    flags | nix::libc::O_NONBLOCK,
                )
            },
            0
        );
        let slave = File::from(pty.slave);
        let stdin = slave.try_clone().unwrap();
        let stdout = slave.try_clone().unwrap();
        let mut child_command = Command::new(assert_cmd::cargo::cargo_bin("fzz"));
        child_command
            .arg("--verbose")
            .current_dir(&root)
            .env("FUNZZY_COLORED", "false")
            .env("PATH", root.join("bin"))
            .env("FUNZZY_NON_BLOCK", if non_block { "true" } else { "false" })
            .stdin(Stdio::from(stdin))
            .stdout(Stdio::from(stdout))
            .stderr(Stdio::from(slave));
        unsafe {
            child_command.pre_exec(|| {
                if nix::libc::setsid() == -1
                    || nix::libc::ioctl(0, nix::libc::TIOCSCTTY as _, 0) == -1
                    || nix::libc::tcsetpgrp(0, nix::libc::getpgrp()) == -1
                {
                    return Err(std::io::Error::last_os_error());
                }
                Ok(())
            });
        }
        let _child = ChildGuard(
            child_command
                .spawn()
                .expect("spawn watcher in foreground pty"),
        );
        let mut output = String::new();
        let result = (|| -> Result<(), String> {
            wait_for(&mut master, &mut output, "Watching...");
            master.write_all(&[0x07]).unwrap();
            wait_for(&mut master, &mut output, "Failure; Completed:");
            master.write_all(&[0x19]).unwrap();
            wait_for(
                &mut master,
                &mut output,
                "Copied latest failed generation output",
            );
            let text = std::fs::read_to_string(&copied).expect("clipboard helper captured stdin");
            assert!(text.contains("Funzzy generation 1 failed"), "{text}");
            assert!(text.contains("clipboard regression"), "{text}");
            assert!(text.contains("stdout-marker"), "{text}");
            assert!(text.contains("stderr-marker"), "{text}");

            std::fs::write(root.join("pass.marker"), "pass").unwrap();
            master.write_all(&[0x07]).unwrap();
            wait_for(&mut master, &mut output, "Success; Completed:");
            master.write_all(&[0x19]).unwrap();
            wait_for(
                &mut master,
                &mut output,
                "There is no current failed generation",
            );
            assert_eq!(std::fs::read_to_string(&copied).unwrap(), text);
            Ok(())
        })();
        result.unwrap_or_else(|error| panic!("watcher test failed: {error}"));
    }
}
