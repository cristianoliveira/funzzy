//! Bounded plain-text failure output copying for foreground watch shortcuts.

use crate::output::RetrievedOutput;
use std::io::{self, Write};
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
    let mut unavailable = Vec::new();
    let mut failures = Vec::new();
    for (program, args) in candidates {
        let spawned = Command::new(program)
            .args(*args)
            .stdin(Stdio::piped())
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .spawn();
        let mut child = match spawned {
            Ok(child) => child,
            Err(error) if error.kind() == io::ErrorKind::NotFound => {
                unavailable.push(*program);
                continue;
            }
            Err(error) => return Err(error),
        };
        let write_result = child
            .stdin
            .take()
            .expect("piped clipboard stdin")
            .write_all(text.as_bytes());
        let status = wait_for_helper(&mut child)?;
        write_result?;
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

fn wait_for_helper(child: &mut Child) -> io::Result<ExitStatus> {
    let deadline = std::time::Instant::now() + std::time::Duration::from_secs(2);
    loop {
        if let Some(status) = child.try_wait()? {
            return Ok(status);
        }
        if std::time::Instant::now() >= deadline {
            let _ = child.kill();
            let _ = child.wait();
            return Err(io::Error::new(
                io::ErrorKind::TimedOut,
                "clipboard helper timed out",
            ));
        }
        std::thread::sleep(std::time::Duration::from_millis(10));
    }
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
        let copied = retained_failure_text(8, &[task.clone()], &registry).unwrap();
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
