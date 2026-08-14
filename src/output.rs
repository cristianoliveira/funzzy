//! Bounded per-generation task-output retention and retrieval (TASK-0045,
//! contract §6).
//!
//! Captured output lives separately from live forwarding and the log file:
//! one pipe read feeds live print, log, and the bounded capture. Memory is
//! globally bounded across generations and tasks by a byte budget with
//! deterministic oldest-generation-first eviction. Truncation is always
//! marked; secrets are never inferred (the socket permission is the security
//! boundary, documented in the retrieval command's help).

use crate::cmd::{CaptureBuffer, CaptureData};
use serde_derive::Serialize;
use std::collections::VecDeque;
use std::sync::Mutex;

/// Global retained-output budget (declared in `capabilities.limits.
/// outputRetentionBytes`): even `--full` retrieval can never exceed it.
pub const OUTPUT_RETENTION_BYTES: usize = 1 << 20;

/// Maximum retained generations regardless of size (metadata bound).
pub const OUTPUT_MAX_GENERATIONS: usize = 128;

/// Default retrieval tail when the CLI omits `--tail`/`--full`.
pub const DEFAULT_RETRIEVAL_TAIL: usize = 40;

/// One retained task's per-stream captures.
#[derive(Clone, Debug)]
pub struct TaskOutput {
    pub task: String,
    pub stdout: CaptureBuffer,
    pub stderr: CaptureBuffer,
}

impl TaskOutput {
    fn retained_bytes(&self) -> usize {
        self.stdout.retained_bytes() as usize + self.stderr.retained_bytes() as usize
    }
}

#[derive(Clone, Debug)]
struct GenerationOutput {
    generation: u64,
    tasks: Vec<TaskOutput>,
    bytes: usize,
}

#[derive(Default)]
struct OutputInner {
    /// Oldest generation first; deterministic eviction pops the front.
    generations: VecDeque<GenerationOutput>,
    total_bytes: usize,
}

/// Global retained-output store shared by the worker (capture sink) and the
/// control server (retrieval + failure evidence). One mutex guards it.
pub struct OutputRegistry {
    inner: Mutex<OutputInner>,
}

impl OutputRegistry {
    pub fn new() -> Self {
        Self {
            inner: Mutex::new(OutputInner::default()),
        }
    }

    /// Records one task's final capture for a generation, then enforces the
    /// global byte budget with oldest-generation-first eviction.
    pub fn record(&self, generation: u64, task: String, data: CaptureData) {
        let mut inner = self.inner.lock().unwrap();
        let bytes = data.stdout.retained_bytes() as usize + data.stderr.retained_bytes() as usize;
        let task_output = TaskOutput {
            task,
            stdout: data.stdout,
            stderr: data.stderr,
        };

        let existing = inner
            .generations
            .iter_mut()
            .find(|entry| entry.generation == generation);
        if let Some(entry) = existing {
            let old_bytes = entry
                .tasks
                .iter()
                .find(|existing| existing.task == task_output.task)
                .map(TaskOutput::retained_bytes)
                .unwrap_or(0);
            entry.bytes = entry.bytes + bytes - old_bytes;
            entry
                .tasks
                .retain(|existing| existing.task != task_output.task);
            entry.tasks.push(task_output);
            inner.total_bytes = inner.total_bytes + bytes - old_bytes;
        } else {
            inner.total_bytes += bytes;
            inner.generations.push_back(GenerationOutput {
                generation,
                tasks: vec![task_output],
                bytes,
            });
        }

        // Deterministic eviction: oldest generation first; also cap the
        // generation count so metadata cannot grow without bound.
        while (inner.total_bytes > OUTPUT_RETENTION_BYTES
            || inner.generations.len() > OUTPUT_MAX_GENERATIONS)
            && !inner.generations.is_empty()
        {
            let evicted = inner.generations.pop_front().expect("non-empty");
            inner.total_bytes = inner.total_bytes.saturating_sub(evicted.bytes);
        }
    }

    /// Retrieves bounded output for one generation (contract §6): optional
    /// task and stream filters, `tail` = last N lines per stream, `full` =
    /// everything retained (still bounded by the global budget).
    pub fn retrieve(
        &self,
        generation: u64,
        task: Option<&str>,
        stream: Option<&str>,
        tail: Option<usize>,
        full: bool,
    ) -> Result<RetrievedOutput, String> {
        let inner = self.inner.lock().unwrap();
        let retained_generations: Vec<u64> = inner
            .generations
            .iter()
            .map(|entry| entry.generation)
            .collect();
        let entry = inner
            .generations
            .iter()
            .find(|entry| entry.generation == generation)
            .ok_or_else(|| {
                let retained = retained_generations;
                if retained.is_empty() {
                    format!(
                        "no retained output for generation {}: nothing is retained (watcher restarted or budget evicted it)",
                        generation
                    )
                } else {
                    format!(
                        "no retained output for generation {}: retained generations are {}",
                        generation,
                        retained
                            .iter()
                            .map(u64::to_string)
                            .collect::<Vec<_>>()
                            .join(", ")
                    )
                }
            })?;

        let tasks: Vec<RetrievedTask> = entry
            .tasks
            .iter()
            .filter(|task_output| {
                task.map(|wanted| task_output.task == wanted)
                    .unwrap_or(true)
            })
            .map(|task_output| RetrievedTask {
                id: task_output.task.clone(),
                stdout: render_stream(&task_output.stdout, stream, tail, full, false),
                stderr: render_stream(&task_output.stderr, stream, tail, full, true),
            })
            .collect();

        if let Some(wanted) = task {
            if !tasks.iter().any(|retrieved| retrieved.id == wanted) {
                let known = entry
                    .tasks
                    .iter()
                    .map(|task_output| task_output.task.clone())
                    .collect::<Vec<_>>()
                    .join(", ");
                return Err(format!(
                    "no retained output for task '{}' in generation {}; retained tasks: {}",
                    wanted, generation, known
                ));
            }
        }

        Ok(RetrievedOutput { generation, tasks })
    }

    /// Concise deterministic failure evidence for a generation's failed
    /// tasks (contract §6): an excerpt up to `max_lines` total lines, the
    /// retained/observed sizes, truncation flags, and the exact retrieval
    /// command. Empty when the generation has no retained output.
    pub fn failure_evidence(&self, generation: u64, max_lines: usize) -> Option<FailureEvidence> {
        let inner = self.inner.lock().unwrap();
        let entry = inner
            .generations
            .iter()
            .find(|entry| entry.generation == generation)?;
        if entry.tasks.is_empty() {
            return None;
        }

        let mut excerpt = String::new();
        let mut lines = 0usize;
        let mut truncated = false;
        let mut total_observed = 0u64;
        let mut total_retained = 0u64;
        let mut first_task: Option<&str> = None;

        for task_output in &entry.tasks {
            let task_name = first_task.get_or_insert(&task_output.task);
            if *task_name != task_output.task {
                continue; // evidence names the first retained task only
            }
            total_observed +=
                task_output.stdout.observed_bytes() + task_output.stderr.observed_bytes();
            total_retained +=
                task_output.stdout.retained_bytes() + task_output.stderr.retained_bytes();
            truncated |= task_output.stdout.truncated() || task_output.stderr.truncated();
            for buffer in [&task_output.stdout, &task_output.stderr] {
                let text = String::from_utf8_lossy(buffer.bytes());
                for line in text.split_inclusive('\n') {
                    if lines >= max_lines {
                        truncated = true;
                        break;
                    }
                    excerpt.push_str(line);
                    lines += 1;
                }
            }
        }

        Some(FailureEvidence {
            excerpt,
            lines: lines as u64,
            truncated,
            total_observed_bytes: total_observed,
            retained_bytes: total_retained,
            retrieve: format!(
                "fzz control output --generation {} --task '{}' --tail 80",
                generation,
                first_task.unwrap_or("")
            ),
        })
    }
}

fn render_stream(
    buffer: &CaptureBuffer,
    stream: Option<&str>,
    tail: Option<usize>,
    full: bool,
    is_stderr: bool,
) -> Option<StreamOutput> {
    match stream {
        Some("stdout") if is_stderr => return None,
        Some("stderr") if !is_stderr => return None,
        Some(other) if other != "stdout" && other != "stderr" => return None,
        _ => {}
    }
    let text = String::from_utf8_lossy(buffer.bytes());
    let content = if full {
        text.to_string()
    } else {
        let take = tail.unwrap_or(DEFAULT_RETRIEVAL_TAIL);
        let lines = text.split_inclusive('\n').collect::<Vec<_>>();
        let kept: Vec<&str> = lines.iter().rev().take(take).rev().copied().collect();
        kept.concat()
    };
    Some(StreamOutput {
        content,
        lines: text.split_inclusive('\n').count() as u64,
        retained_bytes: buffer.retained_bytes(),
        observed_bytes: buffer.observed_bytes(),
        truncated: buffer.truncated(),
    })
}

/// One stream's retrieved output plus its bounds metadata.
#[derive(Clone, Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct StreamOutput {
    pub content: String,
    pub lines: u64,
    pub retained_bytes: u64,
    pub observed_bytes: u64,
    pub truncated: bool,
}

/// One task's retrieved streams.
#[derive(Clone, Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct RetrievedTask {
    pub id: String,
    pub stdout: Option<StreamOutput>,
    pub stderr: Option<StreamOutput>,
}

/// Retrieval domain result (contract §6): consumed by both the structured
/// JSON wire and the human renderer.
#[derive(Clone, Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct RetrievedOutput {
    pub generation: u64,
    pub tasks: Vec<RetrievedTask>,
}

/// Concise failure evidence attached to a failed generation's observation.
#[derive(Clone, Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct FailureEvidence {
    pub excerpt: String,
    pub lines: u64,
    pub truncated: bool,
    pub total_observed_bytes: u64,
    pub retained_bytes: u64,
    pub retrieve: String,
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::cmd::CaptureHandle;
    use std::sync::Arc;

    fn handle_with(lines: &[&str]) -> Arc<CaptureHandle> {
        let handle = Arc::new(CaptureHandle::new());
        for line in lines {
            handle.append(line.as_bytes(), false);
        }
        handle
    }

    fn record(registry: &OutputRegistry, generation: u64, task: &str, lines: &[&str]) {
        registry.record(generation, task.to_owned(), handle_with(lines).finish());
    }

    #[test]
    fn capture_keeps_tail_and_marks_truncation() {
        let handle = Arc::new(CaptureHandle::new());
        // Exceed the per-stream bound with a single huge line.
        let huge = "x".repeat(crate::cmd::CAPTURE_STREAM_BYTES + 100);
        handle.append(huge.as_bytes(), false);
        let data = handle.finish();
        assert!(data.stdout.truncated());
        assert_eq!(data.stdout.observed_bytes() as usize, huge.len());
        assert!(data.stdout.retained_bytes() as usize <= crate::cmd::CAPTURE_STREAM_BYTES);
        assert_eq!(data.stdout.bytes().last(), Some(&b'x'));
    }

    #[test]
    fn capture_separates_streams_and_preserves_partial_lines() {
        let handle = Arc::new(CaptureHandle::new());
        handle.append(b"line one\n", false);
        handle.append(b"partial-no-newline", true);
        let data = handle.finish();
        assert_eq!(data.stdout.bytes(), b"line one\n");
        assert_eq!(data.stderr.bytes(), b"partial-no-newline");
        assert!(!data.stdout.truncated());
    }

    #[test]
    fn non_utf8_bytes_are_retained_raw_and_rendered_lossy() {
        let handle = Arc::new(CaptureHandle::new());
        handle.append(&[0xff, 0xfe, b'\n'], false);
        let data = handle.finish();
        assert_eq!(data.stdout.bytes(), &[0xff, 0xfe, b'\n']);
        let registry = OutputRegistry::new();
        registry.record(1, "t".to_owned(), data);
        let retrieved = registry.retrieve(1, None, None, None, false).unwrap();
        let stdout = retrieved.tasks[0].stdout.as_ref().unwrap();
        assert!(stdout.content.contains('\u{fffd}'), "lossy rendering");
    }

    #[test]
    fn retrieve_tail_returns_last_n_lines() {
        let registry = OutputRegistry::new();
        record(&registry, 1, "t", &["a\n", "b\n", "c\n", "d\n"]);
        let retrieved = registry.retrieve(1, None, None, Some(2), false).unwrap();
        assert_eq!(
            retrieved.tasks[0].stdout.as_ref().unwrap().content,
            "c\nd\n"
        );
    }

    #[test]
    fn retrieve_full_returns_everything_retained() {
        let registry = OutputRegistry::new();
        record(&registry, 1, "t", &["a\n", "b\n"]);
        let retrieved = registry.retrieve(1, None, None, None, true).unwrap();
        assert_eq!(
            retrieved.tasks[0].stdout.as_ref().unwrap().content,
            "a\nb\n"
        );
    }

    #[test]
    fn stream_filter_selects_stderr_only() {
        let registry = OutputRegistry::new();
        registry.record(
            1,
            "t".to_owned(),
            handle_with(&["out\n"]).finish(), // stdout only
        );
        // record with stderr content too
        let handle = Arc::new(CaptureHandle::new());
        handle.append(b"out\n", false);
        handle.append(b"err\n", true);
        registry.record(2, "t".to_owned(), handle.finish());

        let retrieved = registry
            .retrieve(2, None, Some("stderr"), None, false)
            .unwrap();
        assert!(retrieved.tasks[0].stdout.is_none());
        assert_eq!(retrieved.tasks[0].stderr.as_ref().unwrap().content, "err\n");
    }

    #[test]
    fn task_filter_isolates_one_task() {
        let registry = OutputRegistry::new();
        record(&registry, 1, "a", &["aa\n"]);
        record(&registry, 1, "b", &["bb\n"]);
        let retrieved = registry.retrieve(1, Some("b"), None, None, false).unwrap();
        assert_eq!(retrieved.tasks.len(), 1);
        assert_eq!(retrieved.tasks[0].id, "b");
    }

    #[test]
    fn missing_generation_and_task_are_actionable_errors() {
        let registry = OutputRegistry::new();
        record(&registry, 5, "t", &["x\n"]);

        let missing_generation = registry.retrieve(9, None, None, None, false).unwrap_err();
        assert!(
            missing_generation.contains("retained generations are 5"),
            "{}",
            missing_generation
        );

        let missing_task = registry
            .retrieve(5, Some("nope"), None, None, false)
            .unwrap_err();
        assert!(
            missing_task.contains("retained tasks: t"),
            "{}",
            missing_task
        );
    }

    #[test]
    fn eviction_drops_oldest_generation_first() {
        let registry = OutputRegistry::new();
        // Each generation retains ~16 streams x 64 KiB; two generations must
        // exceed the 1 MiB global budget so the oldest is evicted first.
        let big = "y".repeat(crate::cmd::CAPTURE_STREAM_BYTES);
        for generation in 1..=2 {
            for task in 0..16 {
                registry.record(
                    generation,
                    format!("t{task}"),
                    handle_with(&[&big]).finish(),
                );
            }
        }

        assert!(
            registry.retrieve(1, None, None, None, false).is_err(),
            "oldest generation must be evicted first"
        );
        assert!(registry.retrieve(2, None, None, None, false).is_ok());
    }

    #[test]
    fn empty_registry_reports_nothing_retained() {
        let registry = OutputRegistry::new();
        let err = registry.retrieve(1, None, None, None, false).unwrap_err();
        assert!(err.contains("nothing is retained"), "{}", err);
        assert!(registry.failure_evidence(1, 40).is_none());
    }

    #[test]
    fn failure_evidence_is_concise_and_carries_retrieval_hint() {
        let registry = OutputRegistry::new();
        let handle = Arc::new(CaptureHandle::new());
        handle.append(b"error: boom\n", false);
        handle.append(b"detail line\n", false);
        registry.record(7, "my tests".to_owned(), handle.finish());

        let evidence = registry.failure_evidence(7, 40).expect("evidence");
        assert!(evidence.excerpt.contains("error: boom"));
        assert_eq!(evidence.lines, 2);
        assert!(!evidence.truncated);
        assert_eq!(evidence.total_observed_bytes, 24);
        assert!(evidence
            .retrieve
            .contains("--generation 7 --task 'my tests'"));
    }

    #[test]
    fn evidence_excerpt_is_bounded_by_max_lines() {
        let registry = OutputRegistry::new();
        let lines: Vec<&str> = (0..100).map(|_| "line\n").collect();
        record(&registry, 3, "t", &lines);
        let evidence = registry.failure_evidence(3, 40).expect("evidence");
        assert_eq!(evidence.lines, 40);
        assert!(evidence.truncated, "excerpt truncation must be marked");
    }
}
