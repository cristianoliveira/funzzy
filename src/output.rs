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

/// Default serialized page budget (contract §4/§5): conservative below the
/// 64 KiB transport envelope so a page plus JSON escaping and RPC framing
/// never exceeds the agent transport limit.
pub const DEFAULT_PAGE_BYTES: usize = 24 * 1024;

/// Hard cap a client may request per page; the server clamps above this so a
/// malformed or hostile `maxBytes` can never defeat the transport guarantee.
pub const OUTPUT_PAGE_MAX_BYTES: usize = 32 * 1024;

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
    /// Frozen config revision this generation ran under (TASK-0091, AC2);
    /// None for legacy runs that never observe reload.
    revision: Option<u64>,
    /// Non-secret semantic hash of the frozen revision.
    revision_hash: Option<String>,
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

/// Typed retrieval failure (contract §3): each variant maps to one stable
/// RPC error code with structured data, so clients never parse message text.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum RetrievalError {
    /// Generation unknown or evicted; `retained` lists retained generations
    /// (empty = nothing retained). Maps to `-32010`.
    GenerationNotFound { retained: Vec<u64> },
    /// Requested task has no retained output in that generation. `candidates`
    /// are the deterministic canonical exact task IDs (contract §6): when
    /// exactly one exists it may be resolved read-only; more than one is
    /// ambiguous and must never be guessed. Maps to `-32011`.
    TaskNotFound {
        task: String,
        candidates: Vec<String>,
        ambiguous: bool,
    },
    /// Stale or tampered paging cursor (contract §5): wrong generation scope,
    /// unknown task/stream position, or byte offset beyond retained output.
    /// Maps to `-32013`.
    InvalidCursor { reason: String },
}

impl OutputRegistry {
    pub fn new() -> Self {
        Self {
            inner: Mutex::new(OutputInner::default()),
        }
    }

    /// Records one task's final capture for a generation, then enforces the
    /// global byte budget with oldest-generation-first eviction. The frozen
    /// config revision rides the record (TASK-0091, AC2) so `output` can
    /// expose the revision a generation's evidence belongs to.
    pub fn record(
        &self,
        generation: u64,
        task: String,
        data: CaptureData,
        revision: Option<u64>,
        revision_hash: Option<String>,
    ) {
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
                revision,
                revision_hash,
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
    ///
    /// Unknown tasks are self-correcting: an exact match wins; otherwise
    /// deterministic canonical candidates (exact IDs the requested string
    /// prefixes) are computed and a single unambiguous candidate is resolved
    /// read-only (reported via `resolvedTask`). Multiple/zero candidates
    /// return a typed error without guessing.
    pub fn retrieve(
        &self,
        generation: u64,
        task: Option<&str>,
        stream: Option<&str>,
        tail: Option<usize>,
        full: bool,
    ) -> Result<RetrievedOutput, RetrievalError> {
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
            .ok_or(RetrievalError::GenerationNotFound {
                retained: retained_generations,
            })?;

        let wanted = task.map(str::trim).filter(|task| !task.is_empty());
        let exact = wanted.map(|wanted| {
            entry
                .tasks
                .iter()
                .find(|task_output| task_output.task == wanted)
        });

        // Exact match wins outright.
        if let Some(Some(task_output)) = exact {
            return Ok(RetrievedOutput {
                generation,
                revision: entry.revision,
                revision_hash: entry.revision_hash.clone(),
                resolved_task: None,
                tasks: vec![retrieved_task(task_output, stream, tail, full)],
                next_cursor: None,
                returned_bytes: None,
                truncated: None,
            });
        }

        // Whole-generation retrieval: every retained task.
        if let Some(wanted) = wanted {
            // Deterministic canonical candidates: exact task IDs the
            // requested string prefixes (the audit's `run integration` ->
            // `run integration @agent-final` shortening case). Only a
            // single canonical match may be resolved read-only; a
            // non-prefix miss never guesses (contract §6).
            let canonical: Vec<String> = entry
                .tasks
                .iter()
                .map(|task_output| task_output.task.clone())
                .filter(|id| id.starts_with(wanted))
                .collect();
            match canonical.as_slice() {
                // One unambiguous canonical candidate: resolve read-only and
                // report the selected exact ID (contract §6).
                [single] => {
                    let task_output = entry
                        .tasks
                        .iter()
                        .find(|task_output| task_output.task == *single)
                        .expect("candidate derived from retained tasks");
                    return Ok(RetrievedOutput {
                        generation,
                        revision: entry.revision,
                        revision_hash: entry.revision_hash.clone(),
                        resolved_task: Some(single.clone()),
                        tasks: vec![retrieved_task(task_output, stream, tail, full)],
                        next_cursor: None,
                        returned_bytes: None,
                        truncated: None,
                    });
                }
                // Multiple canonical matches: ambiguous, never guess.
                _ if canonical.len() > 1 => {
                    return Err(RetrievalError::TaskNotFound {
                        task: wanted.to_string(),
                        ambiguous: true,
                        candidates: canonical,
                    })
                }
                // Zero canonical matches: typed error listing every retained
                // exact ID so the client can copy the right one. Resolution
                // requires a canonical match; a non-prefix miss never guesses.
                _ => {
                    return Err(RetrievalError::TaskNotFound {
                        task: wanted.to_string(),
                        ambiguous: false,
                        candidates: entry
                            .tasks
                            .iter()
                            .map(|task_output| task_output.task.clone())
                            .collect(),
                    })
                }
            }
        }

        let tasks: Vec<RetrievedTask> = entry
            .tasks
            .iter()
            .map(|task_output| retrieved_task(task_output, stream, tail, full))
            .collect();
        Ok(RetrievedOutput {
            generation,
            revision: entry.revision,
            revision_hash: entry.revision_hash.clone(),
            resolved_task: None,
            tasks,
            next_cursor: None,
            returned_bytes: None,
            truncated: None,
        })
    }

    /// Paged retrieval (contract §5): deterministic ordering by task identity
    /// then stream (stdout before stderr) then byte order; one shared
    /// serialized budget per page so whole-generation retrieval can never
    /// exceed the negotiated agent transport limit regardless of JSON
    /// escaping. The cursor is opaque and validated (`<generation>|<plan
    /// index>|<stream 0|1>|<byte offset>`); stale or tampered cursors yield
    /// [`RetrievalError::InvalidCursor`].
    pub fn retrieve_page(
        &self,
        generation: u64,
        task: Option<&str>,
        stream: Option<&str>,
        max_bytes: usize,
        cursor: Option<&str>,
    ) -> Result<RetrievedOutput, RetrievalError> {
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
            .ok_or(RetrievalError::GenerationNotFound {
                retained: retained_generations,
            })?;

        // Resolve the requested task exactly like tail retrieval: exact match
        // wins, one canonical candidate resolves read-only, ambiguity never
        // guesses.
        let wanted = task.map(str::trim).filter(|task| !task.is_empty());
        let mut resolved_task: Option<String> = None;
        let mut task_ids: Vec<String> = entry
            .tasks
            .iter()
            .map(|task_output| task_output.task.clone())
            .collect();
        if let Some(wanted) = wanted {
            if task_ids.iter().any(|id| id == wanted) {
                task_ids.retain(|id| id == wanted);
            } else {
                let canonical: Vec<String> = task_ids
                    .iter()
                    .filter(|id| id.starts_with(wanted))
                    .cloned()
                    .collect();
                match canonical.as_slice() {
                    [single] => {
                        resolved_task = Some(single.clone());
                        task_ids.retain(|id| id == single);
                    }
                    _ if canonical.len() > 1 => {
                        return Err(RetrievalError::TaskNotFound {
                            task: wanted.to_string(),
                            ambiguous: true,
                            candidates: canonical,
                        })
                    }
                    _ => {
                        return Err(RetrievalError::TaskNotFound {
                            task: wanted.to_string(),
                            ambiguous: false,
                            candidates: task_ids,
                        })
                    }
                }
            }
        }

        // Deterministic page plan (contract §5): ordering is stable by task
        // identity (exact ID, string order) then stream (stdout before
        // stderr), honoring the stream filter — never record/completion order.
        let mut plan: Vec<(String, bool)> = Vec::new(); // (task id, is_stderr)
        let mut sorted_ids = task_ids.clone();
        sorted_ids.sort();
        for task_id in &sorted_ids {
            for is_stderr in [false, true] {
                if let Some(wanted_stream) = stream {
                    if is_stderr != (wanted_stream == "stderr") {
                        continue;
                    }
                }
                plan.push((task_id.clone(), is_stderr));
            }
        }

        // Parse and validate the cursor against this plan: opaque, generation
        // scoped, task/stream/byte positions must exist.
        let start: (usize, usize) = match cursor {
            None => (0, 0),
            Some(raw) => {
                let parts: Vec<&str> = raw.split('|').collect();
                if parts.len() != 4 {
                    return Err(RetrievalError::InvalidCursor {
                        reason: format!(
                            "cursor must be '<gen>|<plan>|<stream>|<offset>', got {raw:?}"
                        ),
                    });
                }
                let (gen, plan_idx, stream_idx, offset) = (
                    parts[0].parse::<u64>(),
                    parts[1].parse::<usize>(),
                    parts[2].parse::<usize>(),
                    parts[3].parse::<usize>(),
                );
                let (gen, plan_idx, stream_idx, offset) = match (gen, plan_idx, stream_idx, offset)
                {
                    (Ok(gen), Ok(plan), Ok(stream), Ok(offset)) => (gen, plan, stream, offset),
                    _ => {
                        return Err(RetrievalError::InvalidCursor {
                            reason: format!("cursor is not numeric: {raw:?}"),
                        })
                    }
                };
                if gen != generation {
                    return Err(RetrievalError::InvalidCursor {
                        reason: format!(
                            "cursor generation {gen} does not match requested generation {generation}"
                        ),
                    });
                }
                if plan_idx >= plan.len() {
                    return Err(RetrievalError::InvalidCursor {
                        reason: format!(
                            "cursor plan index {plan_idx} out of range ({} positions)",
                            plan.len()
                        ),
                    });
                }
                let expected_stream = if plan[plan_idx].1 { 1 } else { 0 };
                if stream_idx != expected_stream {
                    return Err(RetrievalError::InvalidCursor {
                        reason: format!(
                            "cursor stream index {stream_idx} does not match position {plan_idx}"
                        ),
                    });
                }
                let buffer = entry
                    .tasks
                    .iter()
                    .find(|task_output| task_output.task == plan[plan_idx].0)
                    .map(|task_output| {
                        if plan[plan_idx].1 {
                            &task_output.stderr
                        } else {
                            &task_output.stdout
                        }
                    })
                    .expect("task id from retained tasks");
                if offset > buffer.retained_bytes() as usize {
                    return Err(RetrievalError::InvalidCursor {
                        reason: format!(
                            "cursor byte offset {offset} beyond retained {} bytes",
                            buffer.retained_bytes()
                        ),
                    });
                }
                (plan_idx, offset)
            }
        };

        // Walk the plan accumulating content until the measured serialized
        // size would exceed the budget (JSON escaping expands bytes, so the
        // budget is enforced on the serialized document, never on content).
        let mut tasks: Vec<RetrievedTask> = Vec::new();
        let mut returned = 0usize;
        let mut next_cursor: Option<String> = None;

        'walk: for (plan_idx, (task_id, is_stderr)) in plan.iter().enumerate() {
            if plan_idx < start.0 {
                continue;
            }
            let begin = if plan_idx == start.0 { start.1 } else { 0 };
            let task_output = entry
                .tasks
                .iter()
                .find(|task_output| &task_output.task == task_id)
                .expect("task id from retained tasks");
            let buffer = if *is_stderr {
                &task_output.stderr
            } else {
                &task_output.stdout
            };
            if begin >= buffer.bytes().len() {
                continue;
            }
            let text = String::from_utf8_lossy(&buffer.bytes()[begin..]).into_owned();

            // Try to append the whole remaining stream; if the serialized
            // page then exceeds the budget, trim to a char boundary and
            // leave a continuation cursor.
            let mut candidate = tasks.clone();
            set_stream(&mut candidate, task_id, *is_stderr, &text, buffer);
            let page = RetrievedOutput {
                generation,
                revision: entry.revision,
                revision_hash: entry.revision_hash.clone(),
                resolved_task: resolved_task.clone(),
                tasks: candidate,
                next_cursor: Some("cursor".to_owned()),
                returned_bytes: Some((returned + text.len()) as u64),
                truncated: Some(true),
            };
            let serialized = serde_json::to_vec(&page).unwrap_or_default().len();
            if serialized <= max_bytes || text.is_empty() {
                tasks = page.tasks;
                returned += text.len();
                continue;
            }

            // Trim to a char boundary that fits the budget — including the
            // final paging metadata (nextCursor/returnedBytes/truncated) in
            // the measured size. Binary search over char boundaries: the
            // serialized size grows monotonically with the prefix length, so
            // the largest fitting boundary is found in O(log chars) fits.
            let mut boundaries: Vec<usize> = text.char_indices().map(|(i, _)| i).collect();
            boundaries.push(text.len());
            let mut lo = 0usize; // number of leading chars kept (fits)
            let mut hi = boundaries.len(); // exclusive upper bound (may not fit)
            while lo < hi {
                let mid = (lo + hi + 1) / 2;
                let keep = boundaries[mid];
                let mut candidate = tasks.clone();
                set_stream(&mut candidate, task_id, *is_stderr, &text[..keep], buffer);
                let page = RetrievedOutput {
                    generation,
                    revision: entry.revision,
                    revision_hash: entry.revision_hash.clone(),
                    resolved_task: resolved_task.clone(),
                    tasks: candidate,
                    next_cursor: Some("cursor".to_owned()),
                    returned_bytes: Some((returned + keep) as u64),
                    truncated: Some(true),
                };
                if serde_json::to_vec(&page).unwrap_or_default().len() <= max_bytes {
                    lo = mid;
                } else {
                    hi = mid.saturating_sub(1);
                }
            }
            let keep = boundaries[lo];
            if keep > 0 {
                let mut trimmed = tasks.clone();
                set_stream(&mut trimmed, task_id, *is_stderr, &text[..keep], buffer);
                tasks = trimmed;
                returned += keep;
            }
            if keep > 0 || begin + keep < buffer.bytes().len() {
                next_cursor = Some(format!(
                    "{generation}|{plan_idx}|{}|{}",
                    if *is_stderr { 1 } else { 0 },
                    begin + keep
                ));
            }
            break 'walk;
        }

        let truncated = next_cursor.is_some();
        Ok(RetrievedOutput {
            generation,
            revision: entry.revision,
            revision_hash: entry.revision_hash.clone(),
            resolved_task,
            tasks,
            next_cursor,
            returned_bytes: Some(returned as u64),
            truncated: Some(truncated),
        })
    }

    /// Concise deterministic failure evidence for a generation's failed
    /// tasks (contract §6): an excerpt up to `max_lines` total lines, the
    /// retained/observed sizes, truncation flags, and the exact retrieval
    /// command. Empty when the generation has no retained output. `failed_tasks`
    /// names the failed tasks so evidence prefers a failed task (contract §1
    /// §5); when none is retained, the first retained task is the fallback.
    pub fn failure_evidence(
        &self,
        generation: u64,
        max_lines: usize,
        instance_token: &str,
        failed_tasks: &[String],
    ) -> Option<FailureEvidence> {
        let inner = self.inner.lock().unwrap();
        let entry = inner
            .generations
            .iter()
            .find(|entry| entry.generation == generation)?;
        if entry.tasks.is_empty() {
            return None;
        }

        // Prefer the first retained task that actually failed (parallel
        // completion order differs from declaration); fall back to the first
        // retained task when no failed one is retained.
        let mut primary: Option<String> = entry
            .tasks
            .iter()
            .map(|task_output| task_output.task.clone())
            .find(|id| failed_tasks.iter().any(|failed| failed == id));
        if primary.is_none() {
            primary = entry
                .tasks
                .first()
                .map(|task_output| task_output.task.clone());
        }
        let primary = primary.expect("non-empty tasks");

        let mut excerpt = String::new();
        let mut lines = 0usize;
        let mut truncated = false;
        let mut total_observed = 0u64;
        let mut total_retained = 0u64;
        let mut additional = 0u64;

        for task_output in &entry.tasks {
            if task_output.task != primary {
                // Evidence names the primary task only; count the rest so
                // compact status can declare them (contract §5).
                additional += 1;
                continue;
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

        // A structured reference is emitted only when the failed task actually
        // retained output; empty capture never encourages a retrieval.
        let output_ref = if total_retained > 0 && !primary.is_empty() {
            Some(output_ref(instance_token, generation, &primary))
        } else {
            None
        };

        Some(FailureEvidence {
            excerpt,
            lines: lines as u64,
            truncated,
            total_observed_bytes: total_observed,
            retained_bytes: total_retained,
            retrieve: legacy_retrieve_command(generation, &primary),
            output_ref,
            additional_failed_tasks: additional,
        })
    }
}

/// Shell-safe single-quoted task argument: `'` inside the ID becomes `'\''`,
/// the POSIX idiom, so tags, spaces, and quotes never break the command.
fn shell_quote(task: &str) -> String {
    format!("'{}'", task.replace('\'', "'\\''"))
}

/// Legacy human retrieval command (kept for text output; the structured
/// [`OutputRef`] is the machine source of truth).
fn legacy_retrieve_command(generation: u64, task: &str) -> String {
    format!(
        "fzz control output --generation {} --task {} --tail 80",
        generation,
        shell_quote(task)
    )
}

/// Structured exact output reference (contract §1/§5): instance token +
/// generation + exact task identity + safe retrieval defaults, plus a
/// shell-safe command derived from the same identities.
fn output_ref(instance_token: &str, generation: u64, task: &str) -> OutputRef {
    OutputRef {
        instance_token: instance_token.to_owned(),
        generation,
        task: task.to_owned(),
        mode: "tail".to_owned(),
        tail: 80,
        max_bytes: DEFAULT_PAGE_BYTES as u64,
        retrieve: format!(
            "fzz control output --instance {} --generation {} --task {} --tail 80",
            shell_quote(instance_token),
            generation,
            shell_quote(task)
        ),
    }
}

fn retrieved_task(
    task_output: &TaskOutput,
    stream: Option<&str>,
    tail: Option<usize>,
    full: bool,
) -> RetrievedTask {
    RetrievedTask {
        id: task_output.task.clone(),
        stdout: render_stream(&task_output.stdout, stream, tail, full, false),
        stderr: render_stream(&task_output.stderr, stream, tail, full, true),
    }
}

/// Sets one stream's paged content on an existing (or freshly appended) task
/// entry, carrying the stream's bounds metadata (contract §5).
fn set_stream(
    tasks: &mut Vec<RetrievedTask>,
    task_id: &str,
    is_stderr: bool,
    content: &str,
    buffer: &CaptureBuffer,
) {
    if let Some(task) = tasks.iter_mut().find(|task| task.id == task_id) {
        let stream = stream_output(content, buffer);
        if is_stderr {
            task.stderr = Some(stream);
        } else {
            task.stdout = Some(stream);
        }
    } else {
        let mut task = RetrievedTask {
            id: task_id.to_owned(),
            stdout: None,
            stderr: None,
        };
        let stream = stream_output(content, buffer);
        if is_stderr {
            task.stderr = Some(stream);
        } else {
            task.stdout = Some(stream);
        }
        tasks.push(task);
    }
}

/// One stream's bounds metadata for a paged content slice (contract §5): the
/// content is the page's slice; retained/observed/truncated describe the whole
/// captured stream so clients see eviction/truncation across pages.
fn stream_output(content: &str, buffer: &CaptureBuffer) -> StreamOutput {
    StreamOutput {
        content: content.to_owned(),
        lines: content.split_inclusive('\n').count() as u64,
        retained_bytes: buffer.retained_bytes(),
        observed_bytes: buffer.observed_bytes(),
        truncated: buffer.truncated(),
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
    /// Frozen config revision this generation ran under (TASK-0091, AC2);
    /// omitted when the generation predates reload observation.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub revision: Option<u64>,
    /// Non-secret semantic hash of the frozen revision.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub revision_hash: Option<String>,
    /// Exact task ID selected by read-only canonical resolution (contract §6);
    /// absent when the request matched exactly or retrieved whole generation.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub resolved_task: Option<String>,
    pub tasks: Vec<RetrievedTask>,
    /// Paging continuation (contract §5): opaque cursor for the next page,
    /// absent on the final page or non-paged retrieval.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub next_cursor: Option<String>,
    /// Content bytes returned in this page (contract §5).
    #[serde(skip_serializing_if = "Option::is_none")]
    pub returned_bytes: Option<u64>,
    /// True when a continuation exists (contract §5); distinct from per-stream
    /// capture truncation.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub truncated: Option<bool>,
}

/// Concise failure evidence attached to a failed generation's observation.
#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct FailureEvidence {
    pub excerpt: String,
    pub lines: u64,
    pub truncated: bool,
    pub total_observed_bytes: u64,
    pub retained_bytes: u64,
    /// Shell-safe human retrieval command (legacy projection, kept for
    /// humans); the structured [`FailureEvidence::output_ref`] is the
    /// machine source of truth.
    pub retrieve: String,
    /// Exact, copy-safe output reference (contract §1/§5): instance token,
    /// generation, exact task ID, and safe retrieval defaults. Absent when
    /// the failed task retained no useful output.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub output_ref: Option<OutputRef>,
    /// Number of additional failed tasks beyond the named primary, so compact
    /// status can declare "N more failed" without emitting N excerpts.
    pub additional_failed_tasks: u64,
}

/// Structured output reference (contract §1): the agent copies these exact
/// identities instead of reconstructing task names from prose. `retrieve` is
/// a shell-safe command generated from the same identities — tags, spaces,
/// and single quotes in task IDs cannot corrupt it.
#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct OutputRef {
    pub instance_token: String,
    pub generation: u64,
    /// Exact task identity as recorded; never a shortened display name.
    pub task: String,
    /// Retrieval mode (contract §5): always "tail" for evidence defaults.
    pub mode: String,
    /// Safe default tail lines per stream.
    pub tail: u64,
    /// Safe default serialized page budget.
    pub max_bytes: u64,
    /// Shell-safe copy-paste retrieval command derived from the identities.
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
        registry.record(
            generation,
            task.to_owned(),
            handle_with(lines).finish(),
            None,
            None,
        );
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
        registry.record(1, "t".to_owned(), data, None, None);
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
            None,
            None,
        );
        // record with stderr content too
        let handle = Arc::new(CaptureHandle::new());
        handle.append(b"out\n", false);
        handle.append(b"err\n", true);
        registry.record(2, "t".to_owned(), handle.finish(), None, None);

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
    fn missing_generation_and_task_are_typed_errors() {
        let registry = OutputRegistry::new();
        record(&registry, 5, "t", &["x\n"]);

        match registry.retrieve(9, None, None, None, false).unwrap_err() {
            RetrievalError::GenerationNotFound { retained } => {
                assert_eq!(retained, vec![5]);
            }
            other => panic!("expected GenerationNotFound, got {:?}", other),
        }

        match registry
            .retrieve(5, Some("nope"), None, None, false)
            .unwrap_err()
        {
            RetrievalError::TaskNotFound {
                task,
                candidates,
                ambiguous,
            } => {
                assert_eq!(task, "nope");
                assert_eq!(candidates, vec!["t"]);
                assert!(!ambiguous, "single candidate is unambiguous");
            }
            other => panic!("expected TaskNotFound, got {:?}", other),
        }
    }

    #[test]
    fn empty_registry_generation_error_lists_nothing_retained() {
        let registry = OutputRegistry::new();
        match registry.retrieve(1, None, None, None, false).unwrap_err() {
            RetrievalError::GenerationNotFound { retained } => {
                assert!(retained.is_empty());
            }
            other => panic!("expected GenerationNotFound, got {:?}", other),
        }
    }

    #[test]
    fn unambiguous_canonical_task_candidate_is_resolved() {
        let registry = OutputRegistry::new();
        // The audit case: agent sends the shortened "run integration" while
        // the recorded exact identity is "run integration @agent-final".
        record(&registry, 7, "run integration @agent-final", &["boom\n"]);

        let retrieved = registry
            .retrieve(7, Some("run integration"), None, None, false)
            .expect("one unambiguous canonical candidate resolves");
        assert_eq!(retrieved.tasks.len(), 1);
        assert_eq!(retrieved.tasks[0].id, "run integration @agent-final");
        assert_eq!(
            retrieved.resolved_task.as_deref(),
            Some("run integration @agent-final")
        );
    }

    #[test]
    fn ambiguous_canonical_task_candidates_error_without_guessing() {
        let registry = OutputRegistry::new();
        record(&registry, 7, "run integration @agent-final", &["a\n"]);
        record(&registry, 7, "run integration @nightly", &["b\n"]);

        match registry
            .retrieve(7, Some("run integration"), None, None, false)
            .unwrap_err()
        {
            RetrievalError::TaskNotFound {
                task,
                candidates,
                ambiguous,
            } => {
                assert_eq!(task, "run integration");
                assert_eq!(
                    candidates,
                    vec!["run integration @agent-final", "run integration @nightly"]
                );
                assert!(ambiguous, "two candidates is ambiguous");
            }
            other => panic!("expected ambiguous TaskNotFound, got {:?}", other),
        }
    }

    #[test]
    fn exact_task_match_takes_precedence_over_canonical_resolution() {
        let registry = OutputRegistry::new();
        record(&registry, 7, "lint", &["x\n"]);
        record(&registry, 7, "lint @fast", &["y\n"]);

        // Exact "lint" must not resolve to the canonical "lint @fast".
        let retrieved = registry
            .retrieve(7, Some("lint"), None, None, false)
            .unwrap();
        assert_eq!(retrieved.tasks.len(), 1);
        assert_eq!(retrieved.tasks[0].id, "lint");
        assert!(retrieved.resolved_task.is_none());
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
                    None,
                    None,
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
        match registry.retrieve(1, None, None, None, false).unwrap_err() {
            RetrievalError::GenerationNotFound { retained } => {
                assert!(retained.is_empty());
            }
            other => panic!("expected GenerationNotFound, got {:?}", other),
        }
        assert!(registry.failure_evidence(1, 40, "fz-test", &[]).is_none());
    }

    #[test]
    fn failure_evidence_is_concise_and_carries_retrieval_hint() {
        let registry = OutputRegistry::new();
        let handle = Arc::new(CaptureHandle::new());
        handle.append(b"error: boom\n", false);
        handle.append(b"detail line\n", false);
        registry.record(7, "my tests".to_owned(), handle.finish(), None, None);

        let evidence = registry
            .failure_evidence(7, 40, "fz-7f3a", &[])
            .expect("evidence");
        assert!(evidence.excerpt.contains("error: boom"));
        assert_eq!(evidence.lines, 2);
        assert!(!evidence.truncated);
        assert_eq!(evidence.total_observed_bytes, 24);
        assert!(evidence
            .retrieve
            .contains("--generation 7 --task 'my tests'"));
        let output_ref = evidence.output_ref.expect("structured ref");
        assert_eq!(output_ref.instance_token, "fz-7f3a");
        assert_eq!(output_ref.generation, 7);
        assert_eq!(output_ref.task, "my tests");
        assert_eq!(output_ref.mode, "tail");
        assert!(output_ref.max_bytes > 0);
        assert!(output_ref.retrieve.contains("--instance 'fz-7f3a'"));
    }

    #[test]
    fn output_ref_retrieve_command_is_shell_safe_with_quotes_and_spaces() {
        let registry = OutputRegistry::new();
        let handle = Arc::new(CaptureHandle::new());
        handle.append(b"boom\n", false);
        // Task IDs may carry tags, spaces, and single quotes; the generated
        // command must survive copy-paste without corruption (contract §5).
        registry.record(
            7,
            "run integration @agent-final it's 'quoted'".to_owned(),
            handle.finish(),
            None,
            None,
        );

        let evidence = registry
            .failure_evidence(7, 40, "fz-7f3a", &[])
            .expect("evidence");
        let command = &evidence.output_ref.expect("ref").retrieve;
        assert!(
            command.contains("--task '"),
            "task must be single-quoted: {command}"
        );
        assert!(
            command.matches("'\\''").count() >= 2,
            "single quotes inside the task must be escaped as '\\'' : {command}"
        );
        assert!(command.contains(" --instance 'fz-7f3a'"), "{command}");
        assert!(command.contains(" --tail 80"), "{command}");
    }

    #[test]
    fn evidence_counts_additional_failed_tasks_while_naming_primary() {
        let registry = OutputRegistry::new();
        record(&registry, 7, "lint", &["lint boom\n"]);
        record(&registry, 7, "test", &["test boom\n"]);

        let evidence = registry
            .failure_evidence(7, 40, "fz-7f3a", &[])
            .expect("evidence");
        // Primary is the first retained task; the additional count declares
        // the rest without emitting a second excerpt (contract §5).
        assert_eq!(evidence.output_ref.as_ref().unwrap().task, "lint");
        assert_eq!(evidence.additional_failed_tasks, 1);
        // retained_bytes reflects the primary excerpt only; the additional
        // task's bytes are declared via the count, never a second excerpt.
        assert_eq!(evidence.retained_bytes, 10); // "lint boom\n"
    }

    #[test]
    fn evidence_prefers_a_failed_task_over_first_retained() {
        let registry = OutputRegistry::new();
        // "slow pass" is recorded first (completes first), "fast fail"
        // second; evidence must name the failed task, not the first retained.
        record(&registry, 7, "slow pass", &["slow ok\n"]);
        record(&registry, 7, "fast fail", &["early boom\n"]);

        let evidence = registry
            .failure_evidence(7, 40, "fz-7f3a", &["fast fail".to_owned()])
            .expect("evidence");
        assert_eq!(evidence.output_ref.as_ref().unwrap().task, "fast fail");
        assert!(evidence.excerpt.contains("early boom"));
        assert_eq!(evidence.additional_failed_tasks, 1);

        // No failed task retained → first retained is the deterministic
        // fallback (contract §5).
        let fallback = registry
            .failure_evidence(7, 40, "fz-7f3a", &["nope".to_owned()])
            .expect("evidence");
        assert_eq!(fallback.output_ref.as_ref().unwrap().task, "slow pass");
    }

    #[test]
    fn evidence_without_output_has_no_ref_and_zero_additional() {
        let registry = OutputRegistry::new();
        let handle = Arc::new(CaptureHandle::new());
        handle.append(b"", false);
        registry.record(7, "t".to_owned(), handle.finish(), None, None);
        let evidence = registry
            .failure_evidence(7, 40, "fz-7f3a", &[])
            .expect("evidence");
        assert!(evidence.output_ref.is_none());
        assert_eq!(evidence.additional_failed_tasks, 0);
    }

    #[test]
    fn evidence_excerpt_is_bounded_by_max_lines() {
        let registry = OutputRegistry::new();
        let lines: Vec<&str> = (0..100).map(|_| "line\n").collect();
        record(&registry, 3, "t", &lines);
        let evidence = registry
            .failure_evidence(3, 40, "fz-test", &[])
            .expect("evidence");
        assert_eq!(evidence.lines, 40);
        assert!(evidence.truncated, "excerpt truncation must be marked");
    }

    // ---- paging (TASK-0081, contract §5) ----

    #[test]
    fn page_returns_first_chunk_with_next_cursor() {
        let registry = OutputRegistry::new();
        record(&registry, 7, "t", &["aaa\n", "bbb\n", "ccc\n"]);
        // Budget derived from a reference page containing exactly the first
        // two lines, so the page must stop there with a continuation cursor.
        let two_lines = registry
            .retrieve_page(7, None, None, usize::MAX, None)
            .expect("unbounded first page");
        let _ = two_lines; // sanity: unbounded returns everything
        let budget = serde_json::to_vec(&RetrievedOutput {
            generation: 7,
            revision: None,
            revision_hash: None,
            resolved_task: None,
            tasks: vec![RetrievedTask {
                id: "t".to_owned(),
                stdout: Some(StreamOutput {
                    content: "aaa\nbbb\n".to_owned(),
                    lines: 2,
                    retained_bytes: 12,
                    observed_bytes: 12,
                    truncated: false,
                }),
                stderr: None,
            }],
            next_cursor: Some("cursor".to_owned()),
            returned_bytes: Some(0),
            truncated: Some(true),
        })
        .expect("serialize reference")
        .len();

        let page = registry
            .retrieve_page(7, None, None, budget, None)
            .expect("first page");
        let stdout = page.tasks[0].stdout.as_ref().expect("stdout");
        assert_eq!(stdout.content, "aaa\nbbb\n");
        assert!(page.next_cursor.is_some(), "continuation expected");
        assert_eq!(page.truncated, Some(true));
    }

    #[test]
    fn page_resumes_exactly_from_cursor_without_skip_or_duplicate() {
        let registry = OutputRegistry::new();
        record(&registry, 7, "t", &["aaa\n", "bbb\n", "ccc\n"]);
        let budget = serde_json::to_vec(&RetrievedOutput {
            generation: 7,
            revision: None,
            revision_hash: None,
            resolved_task: None,
            tasks: vec![RetrievedTask {
                id: "t".to_owned(),
                stdout: Some(StreamOutput {
                    content: "aaa\nbbb\n".to_owned(),
                    lines: 2,
                    retained_bytes: 12,
                    observed_bytes: 12,
                    truncated: false,
                }),
                stderr: None,
            }],
            next_cursor: Some("cursor".to_owned()),
            returned_bytes: Some(0),
            truncated: Some(true),
        })
        .expect("serialize reference")
        .len();
        let first = registry
            .retrieve_page(7, None, None, budget, None)
            .expect("first page");
        let cursor = first.next_cursor.clone().expect("cursor");
        let second = registry
            .retrieve_page(7, None, None, budget, Some(&cursor))
            .expect("second page");
        let stdout = second.tasks[0].stdout.as_ref().expect("stdout");
        assert_eq!(stdout.content, "ccc\n");
        assert!(second.next_cursor.is_none(), "no more pages");
    }

    #[test]
    fn page_orders_by_task_then_stdout_then_stderr() {
        let registry = OutputRegistry::new();
        let handle = Arc::new(CaptureHandle::new());
        handle.append(b"out-b\n", false);
        handle.append(b"err-b\n", true);
        registry.record(7, "b".to_owned(), handle.finish(), None, None);
        record(&registry, 7, "a", &["out-a\n"]);

        // Budget derived from a reference page with only task "a" stdout:
        // ordering must pick task "a" first (recorded order), never task "b"
        // and never stderr before stdout.
        let budget = serde_json::to_vec(&RetrievedOutput {
            generation: 7,
            revision: None,
            revision_hash: None,
            resolved_task: None,
            tasks: vec![RetrievedTask {
                id: "a".to_owned(),
                stdout: Some(StreamOutput {
                    content: "out-a\n".to_owned(),
                    lines: 1,
                    retained_bytes: 6,
                    observed_bytes: 6,
                    truncated: false,
                }),
                stderr: None,
            }],
            next_cursor: Some("cursor".to_owned()),
            returned_bytes: Some(0),
            truncated: Some(true),
        })
        .expect("serialize reference")
        .len();
        let page = registry
            .retrieve_page(7, None, None, budget, None)
            .expect("page");
        let contents: Vec<String> = page
            .tasks
            .iter()
            .flat_map(|task| {
                [
                    task.stdout.as_ref().map(|s| s.content.clone()),
                    task.stderr.as_ref().map(|s| s.content.clone()),
                ]
            })
            .flatten()
            .collect();
        assert_eq!(contents, vec!["out-a\n".to_owned()]);
        assert_eq!(page.tasks[0].id, "a");
        assert!(page.next_cursor.is_some());
    }

    #[test]
    fn page_never_splits_utf8_mid_char_and_reports_retained_observed() {
        let registry = OutputRegistry::new();
        let handle = Arc::new(CaptureHandle::new());
        // "é" is two bytes; budget derived from a reference page with one
        // "é" must not split the multi-byte char.
        handle.append("ééé\n".as_bytes(), false);
        registry.record(7, "t".to_owned(), handle.finish(), None, None);

        let budget = serde_json::to_vec(&RetrievedOutput {
            generation: 7,
            revision: None,
            revision_hash: None,
            resolved_task: None,
            tasks: vec![RetrievedTask {
                id: "t".to_owned(),
                stdout: Some(StreamOutput {
                    content: "é".to_owned(),
                    lines: 1,
                    retained_bytes: 7,
                    observed_bytes: 7,
                    truncated: false,
                }),
                stderr: None,
            }],
            next_cursor: Some("cursor".to_owned()),
            returned_bytes: Some(0),
            truncated: Some(true),
        })
        .expect("serialize reference")
        .len();
        let page = registry
            .retrieve_page(7, None, None, budget, None)
            .expect("page");
        let stdout = page.tasks[0].stdout.as_ref().expect("stdout");
        assert!(std::str::from_utf8(stdout.content.as_bytes()).is_ok());
        assert_eq!(stdout.content, "é");
        assert!(page.next_cursor.is_some());
        assert_eq!(stdout.retained_bytes, 7); // "ééé\n"
        assert_eq!(stdout.observed_bytes, 7);
    }

    #[test]
    fn page_serialized_response_never_exceeds_effective_budget() {
        let registry = OutputRegistry::new();
        // Pathological escaping: quotes, backslashes, and control chars all
        // expand in JSON; the page builder must trim to the measured budget.
        let nasty: String = (0..4000).map(|_| "\"\\\n\u{0}\u{1}").collect();
        let handle = Arc::new(CaptureHandle::new());
        handle.append(nasty.as_bytes(), false);
        registry.record(7, "t".to_owned(), handle.finish(), None, None);

        let budget = 16 * 1024;
        let page = registry
            .retrieve_page(7, None, None, budget, None)
            .expect("page");
        let serialized = serde_json::to_vec(&page).expect("serialize");
        assert!(
            serialized.len() <= budget,
            "serialized {} > budget {}",
            serialized.len(),
            budget
        );
        assert!(page.next_cursor.is_some(), "continuation after trim");
    }

    #[test]
    fn page_unknown_generation_and_task_reuse_typed_errors() {
        let registry = OutputRegistry::new();
        record(&registry, 5, "t", &["x\n"]);
        match registry.retrieve_page(9, None, None, 8, None).unwrap_err() {
            RetrievalError::GenerationNotFound { retained } => {
                assert_eq!(retained, vec![5]);
            }
            other => panic!("expected GenerationNotFound, got {:?}", other),
        }
        match registry
            .retrieve_page(5, Some("nope"), None, 8, None)
            .unwrap_err()
        {
            RetrievalError::TaskNotFound { candidates, .. } => {
                assert_eq!(candidates, vec!["t"]);
            }
            other => panic!("expected TaskNotFound, got {:?}", other),
        }
    }

    #[test]
    fn page_stale_or_tampered_cursor_is_typed_invalid() {
        let registry = OutputRegistry::new();
        record(&registry, 7, "t", &["aaa\n", "bbb\n"]);
        for bad in [
            "",
            "7|9|0|0",      // task index out of range
            "7|0|5|0",      // stream index out of range
            "7|0|0|999999", // byte offset beyond retained
            "8|0|0|0",      // generation mismatch
            "not-a-cursor",
        ] {
            match registry
                .retrieve_page(7, None, None, 8, Some(bad))
                .unwrap_err()
            {
                RetrievalError::InvalidCursor { .. } => {}
                other => panic!("cursor {bad:?}: expected InvalidCursor, got {:?}", other),
            }
        }
    }

    #[test]
    fn page_task_filter_and_canonical_resolution_apply() {
        let registry = OutputRegistry::new();
        record(&registry, 7, "run integration @agent-final", &["boom\n"]);
        // Shortened name resolves canonically exactly once (contract §6);
        // budget far above the envelope so the whole stream fits one page.
        let page = registry
            .retrieve_page(7, Some("run integration"), None, 4096, None)
            .expect("canonical resolution");
        assert_eq!(
            page.resolved_task.as_deref(),
            Some("run integration @agent-final")
        );
        assert_eq!(page.tasks.len(), 1);
        assert!(page.next_cursor.is_none());
    }
}
