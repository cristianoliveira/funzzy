//! Bounded plain-text failure output copying for foreground watch shortcuts.

use crate::output::RetrievedOutput;
use std::io::{self, Write};
use std::path::{Path, PathBuf};
use std::process::{Child, Command, ExitStatus, Stdio};

pub const MAX_CLIPBOARD_BYTES: usize = 64 * 1024;

pub trait ClipboardWriter {
    fn write_text(&self, text: &str) -> io::Result<()>;
}

pub struct SystemClipboardWriter;

impl ClipboardWriter for SystemClipboardWriter {
    fn write_text(&self, text: &str) -> io::Result<()> {
        copy_to_system(text)
    }
}

/// Refuse to call the clipboard unless a foreground TTY and current failure
/// output are both available.
pub fn copy_latest(
    writer: &dyn ClipboardWriter,
    foreground_tty: bool,
    latest_output: Result<String, String>,
) -> Result<(), String> {
    if !foreground_tty {
        return Err("Ctrl-Y requires a foreground terminal; clipboard unchanged.".to_owned());
    }
    let text = latest_output?;
    writer
        .write_text(&text)
        .map_err(|error| format!("clipboard write failed: {error}"))
}

/// Build copy text from only failed task IDs in the latest failed state.
pub fn retained_failure_text(
    generation: u64,
    snapshots: &[crate::task_result::TaskSnapshot],
    outputs: &crate::output::OutputRegistry,
) -> Result<String, String> {
    let failed: Vec<_> = snapshots
        .iter()
        .filter(|task| {
            matches!(
                task.state,
                crate::task_result::TaskState::Failed | crate::task_result::TaskState::TimedOut
            )
        })
        .collect();
    if failed.is_empty() {
        return Err(
            "The latest failed generation has no failed task output; clipboard unchanged."
                .to_owned(),
        );
    }
    let mut retrieved = Vec::new();
    for task in failed {
        let output = outputs
            .retrieve(generation, Some(&task.name), None, Some(80), false)
            .map_err(|_| {
                format!(
                    "No retained output for failed task {}; clipboard unchanged.",
                    task.name
                )
            })?;
        retrieved.extend(output.tasks);
    }
    let output = crate::output::RetrievedOutput {
        generation,
        revision: None,
        revision_hash: None,
        resolved_task: None,
        tasks: retrieved,
        next_cursor: None,
        returned_bytes: None,
        truncated: None,
    };
    format_failure(generation, output)
        .ok_or_else(|| "No retained failure output; clipboard unchanged.".to_owned())
}

/// Format retained streams with task and generation identity, capped by bytes.
pub fn format_failure(generation: u64, output: RetrievedOutput) -> Option<String> {
    let mut text = format!("Funzzy generation {generation} failed\n");
    let mut has_output = false;
    for task in output.tasks {
        text.push_str(&format!("\n--- {} ---\n", task.id));
        for (name, stream) in [("stdout", task.stdout), ("stderr", task.stderr)] {
            if let Some(stream) = stream.filter(|stream| !stream.content.is_empty()) {
                has_output = true;
                text.push_str(&format!("[{name}]\n"));
                text.push_str(&stream.content);
                if !stream.content.ends_with('\n') {
                    text.push('\n');
                }
            }
        }
    }
    if !has_output {
        return None;
    }
    const MARKER: &str = "\n…(clipboard output truncated)\n";
    if text.len() > MAX_CLIPBOARD_BYTES {
        let mut end = MAX_CLIPBOARD_BYTES - MARKER.len();
        while !text.is_char_boundary(end) {
            end -= 1;
        }
        text.truncate(end);
        text.push_str(MARKER);
    }
    Some(text)
}

/// Copy text using a platform clipboard utility, passing content only on stdin.
pub fn copy_to_system(text: &str) -> io::Result<()> {
    let candidates: &[(&str, &[&str])] = clipboard_commands();
    let workspace = project_root()?;
    let search_path = std::env::var_os("PATH").unwrap_or_default();
    let mut unavailable = Vec::new();
    let mut failures = Vec::new();
    for (program, args) in candidates {
        let Some(executable) = resolve_helper(program, &search_path, &workspace) else {
            unavailable.push(*program);
            continue;
        };
        let mut command = Command::new(executable);
        command
            .args(*args)
            .stdin(Stdio::piped())
            .stdout(Stdio::null())
            .stderr(Stdio::null());
        isolate_helper(&mut command);
        let child = command.spawn()?;
        let status = match write_and_wait(child, text.as_bytes(), std::time::Duration::from_secs(2))
        {
            Ok(status) => status,
            Err(error) if error.kind() == io::ErrorKind::TimedOut => return Err(error),
            Err(error) => {
                failures.push(format!("{program}: {error}"));
                continue;
            }
        };
        if status.success() {
            return Ok(());
        }
        failures.push(format!("{program} exited with {status}"));
    }
    if failures.is_empty() {
        Err(io::Error::new(
            io::ErrorKind::NotFound,
            format!(
                "no supported clipboard helper found ({})",
                unavailable.join(", ")
            ),
        ))
    } else {
        Err(io::Error::other(format!(
            "all clipboard helpers failed: {}",
            failures.join("; ")
        )))
    }
}

fn project_root() -> io::Result<PathBuf> {
    let current = std::env::current_dir()?.canonicalize()?;
    Ok(current
        .ancestors()
        .find(|ancestor| ancestor.join(".git").exists())
        .unwrap_or(&current)
        .to_path_buf())
}

fn resolve_helper(
    program: &str,
    search_path: &std::ffi::OsStr,
    workspace: &Path,
) -> Option<PathBuf> {
    let workspace = workspace.canonicalize().ok()?;
    for entry in std::env::split_paths(search_path) {
        if !entry.is_absolute() {
            continue;
        }
        let Ok(directory) = entry.canonicalize() else {
            continue;
        };
        if directory.starts_with(&workspace) {
            continue;
        }
        let Ok(executable) = directory.join(program).canonicalize() else {
            continue;
        };
        if executable.starts_with(&workspace) || !is_executable(&executable) {
            continue;
        }
        return Some(executable);
    }
    None
}

#[cfg(unix)]
fn is_executable(path: &Path) -> bool {
    use std::os::unix::fs::PermissionsExt;
    path.metadata()
        .map(|metadata| metadata.permissions().mode() & 0o111 != 0)
        .unwrap_or(false)
}

#[cfg(not(unix))]
fn is_executable(path: &Path) -> bool {
    path.is_file()
}

fn write_and_wait(
    mut child: Child,
    bytes: &[u8],
    timeout: std::time::Duration,
) -> io::Result<ExitStatus> {
    let mut stdin = child.stdin.take().expect("piped clipboard stdin");
    let (writer_tx, writer_rx) = std::sync::mpsc::channel();
    let bytes = bytes.to_vec();
    let writer = std::thread::spawn(move || {
        let result = stdin.write_all(&bytes);
        let _ = writer_tx.send(result);
    });
    let deadline = std::time::Instant::now() + timeout;
    let mut status = None;
    let mut write_result = None;
    loop {
        if write_result.is_none() {
            match writer_rx.try_recv() {
                Ok(result) => write_result = Some(result),
                Err(std::sync::mpsc::TryRecvError::Disconnected) => {
                    stop_helper(&mut child);
                    let _ = writer.join();
                    return Err(io::Error::other(
                        "clipboard input writer stopped unexpectedly",
                    ));
                }
                Err(std::sync::mpsc::TryRecvError::Empty) => {}
            }
        }
        if status.is_none() {
            match child.try_wait() {
                Ok(status_now) => status = status_now,
                Err(error) => {
                    stop_helper(&mut child);
                    let _ = writer.join();
                    return Err(error);
                }
            }
        }
        if let Some(Err(error)) = write_result.as_ref() {
            let error = io::Error::new(error.kind(), error.to_string());
            stop_helper(&mut child);
            let _ = writer.join();
            return Err(error);
        }
        if let (Some(status), Some(Ok(()))) = (status.as_ref(), write_result.as_ref()) {
            writer
                .join()
                .map_err(|_| io::Error::other("clipboard input writer panicked"))?;
            return Ok(*status);
        }
        if std::time::Instant::now() >= deadline {
            stop_helper(&mut child);
            let _ = writer.join();
            return Err(io::Error::new(
                io::ErrorKind::TimedOut,
                "clipboard helper timed out",
            ));
        }
        std::thread::sleep(std::time::Duration::from_millis(10));
    }
}

fn isolate_helper(command: &mut Command) {
    #[cfg(unix)]
    {
        use std::os::unix::process::CommandExt;
        unsafe {
            command.pre_exec(|| {
                if nix::libc::setsid() == -1 {
                    return Err(io::Error::last_os_error());
                }
                Ok(())
            });
        }
    }
}

fn stop_helper(child: &mut Child) {
    #[cfg(unix)]
    unsafe {
        let _ = nix::libc::kill(-(child.id() as nix::libc::pid_t), nix::libc::SIGKILL);
    }
    let _ = child.kill();
    let _ = child.wait();
}

#[cfg(target_os = "macos")]
fn clipboard_commands() -> &'static [(&'static str, &'static [&'static str])] {
    &[("pbcopy", &[])]
}

#[cfg(target_os = "linux")]
fn clipboard_commands() -> &'static [(&'static str, &'static [&'static str])] {
    &[
        ("wl-copy", &[]),
        ("xclip", &["-selection", "clipboard"]),
        ("xsel", &["--clipboard", "--input"]),
    ]
}

#[cfg(target_os = "windows")]
fn clipboard_commands() -> &'static [(&'static str, &'static [&'static str])] {
    &[("clip.exe", &[])]
}

#[cfg(not(any(target_os = "macos", target_os = "linux", target_os = "windows")))]
fn clipboard_commands() -> &'static [(&'static str, &'static [&'static str])] {
    &[]
}

#[cfg(test)]
mod tests {
    use super::*;

    #[derive(Default)]
    struct FakeClipboard(std::sync::Mutex<Vec<String>>);

    impl ClipboardWriter for FakeClipboard {
        fn write_text(&self, text: &str) -> io::Result<()> {
            self.0.lock().unwrap().push(text.to_owned());
            Ok(())
        }
    }

    #[test]
    fn helper_resolution_skips_programs_inside_the_project() {
        let root = std::env::temp_dir().join(format!("fzz-clipboard-path-{}", std::process::id()));
        let local = root.join("repo/bin");
        let trusted = root.join("system/bin");
        std::fs::create_dir_all(&local).unwrap();
        std::fs::create_dir_all(&trusted).unwrap();
        std::fs::write(local.join("wl-copy"), "local").unwrap();
        std::fs::write(trusted.join("wl-copy"), "system").unwrap();
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            for path in [local.join("wl-copy"), trusted.join("wl-copy")] {
                let mut permissions = path.metadata().unwrap().permissions();
                permissions.set_mode(0o755);
                std::fs::set_permissions(path, permissions).unwrap();
            }
        }

        assert_eq!(
            resolve_helper("wl-copy", local.as_os_str(), &root.join("repo")),
            None
        );
        assert_eq!(
            resolve_helper(
                "wl-copy",
                &std::env::join_paths([&local, &trusted]).unwrap(),
                &root.join("repo")
            ),
            Some(trusted.join("wl-copy").canonicalize().unwrap())
        );
        let _ = std::fs::remove_dir_all(root);
    }

    #[cfg(unix)]
    #[test]
    fn stalled_helper_with_unread_pipe_is_killed_within_timeout() {
        let mut command = Command::new("sleep");
        command
            .arg("30")
            .stdin(Stdio::piped())
            .stdout(Stdio::null())
            .stderr(Stdio::null());
        isolate_helper(&mut command);
        let child = command.spawn().unwrap();
        let pid = child.id();
        let started = std::time::Instant::now();
        let result = write_and_wait(
            child,
            &vec![b'x'; 128 * 1024],
            std::time::Duration::from_millis(150),
        );
        assert_eq!(result.unwrap_err().kind(), io::ErrorKind::TimedOut);
        assert!(started.elapsed() < std::time::Duration::from_secs(2));
        assert_eq!(unsafe { nix::libc::kill(pid as nix::libc::pid_t, 0) }, -1);
    }

    #[test]
    fn copy_is_skipped_without_tty_or_current_failure() {
        let clipboard = FakeClipboard::default();
        assert!(copy_latest(&clipboard, false, Ok("failure".into())).is_err());
        assert!(copy_latest(&clipboard, true, Err("no failure".into())).is_err());
        assert!(clipboard.0.lock().unwrap().is_empty());
    }

    #[test]
    fn copy_writes_current_failure_text_and_surfaces_backend_errors() {
        let clipboard = FakeClipboard::default();
        copy_latest(&clipboard, true, Ok("generation 3 failed".into())).unwrap();
        assert_eq!(
            clipboard.0.lock().unwrap().as_slice(),
            ["generation 3 failed"]
        );
        struct FailingClipboard;
        impl ClipboardWriter for FailingClipboard {
            fn write_text(&self, _: &str) -> io::Result<()> {
                Err(io::Error::other("unavailable"))
            }
        }
        assert!(copy_latest(&FailingClipboard, true, Ok("failure".into()))
            .unwrap_err()
            .contains("unavailable"));
    }

    #[test]
    fn retained_output_selects_only_failed_tasks_and_rejects_success() {
        let registry = crate::output::OutputRegistry::new();
        let capture = crate::cmd::CaptureHandle::new();
        capture.append(b"error details\n", true);
        registry.record(8, "lint".into(), capture.finish(), None, None);
        let task = crate::task_result::TaskSnapshot {
            position: 0,
            id: "lint".into(),
            name: "lint".into(),
            state: crate::task_result::TaskState::Failed,
            duration_ms: None,
        };
        let copied = retained_failure_text(8, std::slice::from_ref(&task), &registry).unwrap();
        assert!(copied.contains("generation 8 failed"));
        assert!(copied.contains("[stderr]\nerror details"));

        let mut passed = task;
        passed.state = crate::task_result::TaskState::Passed;
        assert!(retained_failure_text(8, &[passed], &registry)
            .unwrap_err()
            .contains("clipboard unchanged"));
        assert!(retained_failure_text(9, &[], &registry).is_err());
    }

    #[test]
    fn formats_generation_task_and_each_stream() {
        let output = RetrievedOutput {
            generation: 7,
            revision: None,
            revision_hash: None,
            resolved_task: None,
            tasks: vec![crate::output::RetrievedTask {
                id: "lint".into(),
                stdout: Some(crate::output::StreamOutput {
                    content: "out\n".into(),
                    lines: 1,
                    retained_bytes: 4,
                    observed_bytes: 4,
                    truncated: false,
                }),
                stderr: Some(crate::output::StreamOutput {
                    content: "err\n".into(),
                    lines: 1,
                    retained_bytes: 4,
                    observed_bytes: 4,
                    truncated: false,
                }),
            }],
            next_cursor: None,
            returned_bytes: None,
            truncated: None,
        };
        let text = format_failure(7, output).unwrap();
        assert!(text.contains("generation 7 failed"));
        assert!(text.contains("--- lint ---"));
        assert!(text.contains("[stdout]\nout"));
        assert!(text.contains("[stderr]\nerr"));
    }

    #[test]
    fn formats_never_exceed_clipboard_bound() {
        let huge = "x".repeat(MAX_CLIPBOARD_BYTES * 2);
        let output = RetrievedOutput {
            generation: 1,
            revision: None,
            revision_hash: None,
            resolved_task: None,
            tasks: vec![crate::output::RetrievedTask {
                id: "task".into(),
                stdout: Some(crate::output::StreamOutput {
                    content: huge,
                    lines: 1,
                    retained_bytes: 0,
                    observed_bytes: 0,
                    truncated: false,
                }),
                stderr: None,
            }],
            next_cursor: None,
            returned_bytes: None,
            truncated: None,
        };
        assert!(format_failure(1, output).unwrap().len() <= MAX_CLIPBOARD_BYTES);
    }
}
