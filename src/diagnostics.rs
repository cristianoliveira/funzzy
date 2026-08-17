//! Typed lifecycle diagnostics (TASK-0023).
//!
//! Replaces ad-hoc verbose dumps with one deterministic record per decision:
//! event batch, source, event kind, path, normalized path, matched task and
//! effective rule, scheduled generation, command outcome, duration, and
//! cancellation reason. Records render as stable `key=value` lines to the
//! terminal and the optional log file with identical semantics.
//!
//! The feedback-loop heuristic is observational only: it can never alter
//! scheduling, cancellation, or task results. It correlates repeated
//! triggers for the same task/path/rule within a bounded time window and
//! emits a `possible feedback loop` warning that names the task, repeated
//! path/rule, repeat count, and related generation — never claiming that a
//! child command caused the filesystem event.

use std::collections::HashMap;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{LazyLock, Mutex};
use std::time::{Duration, Instant};

/// One typed diagnostic record. Every field is optional; the renderer emits
/// only present fields in a fixed, deterministic order.
#[derive(Debug, Clone, Default)]
pub struct Record {
    /// Monotonic diagnostic sequence, assigned by the sink. Orders records
    /// for correlation across batches and generations.
    pub seq: Option<u64>,
    /// Debounce batch identity the record belongs to.
    pub batch: Option<u64>,
    /// Record source: `init`, `filesystem`, `control`, or `config`.
    pub source: Option<&'static str>,
    /// Filesystem event kind as observed: `any` or `continuous`.
    pub kind: Option<&'static str>,
    /// Raw event path as reported by the watcher.
    pub path: Option<String>,
    /// Root-normalized path used for rule matching.
    pub normalized: Option<String>,
    /// Decision taken: `watch_root`, `event`, `matched`, `ignored`,
    /// `unmatched`, `scheduled`, `noop`, `error`.
    pub decision: Option<&'static str>,
    /// Task name the decision concerns.
    pub task: Option<String>,
    /// Effective change rule responsible for a match.
    pub change: Option<String>,
    /// Effective ignore rule responsible for an ignore.
    pub ignore: Option<String>,
    /// Where the effective rule comes from: `task` or `group` (inherited).
    pub rule_origin: Option<String>,
    /// Generation identity of the scheduled run.
    pub generation: Option<u64>,
    /// Busy-run policy of the executing strategy: `restart` or `wait`.
    pub policy: Option<&'static str>,
    /// Total commands scheduled in the run.
    pub commands: Option<usize>,
    /// Command position within its run, rendered as `index/total`.
    pub command_position: Option<(usize, usize)>,
    /// Run or command state: `started`, `passed`, `failed`, `cancelled`.
    pub state: Option<&'static str>,
    /// Rendered command line.
    pub command: Option<String>,
    /// Formatted wall duration, e.g. `0.842s`.
    pub duration: Option<String>,
    /// Cancellation reason: `replaced` or `requested`.
    pub reason: Option<String>,
    /// Generation a later event was observed after (loop correlation).
    pub observed_after_run: Option<u64>,
    /// Repeat count of a trigger chain.
    pub repeats: Option<u64>,
    /// Actionable hint for a loop warning.
    pub hint: Option<String>,
    /// Free-form note (e.g. malformed watcher event detail).
    pub note: Option<String>,
}

/// Renders the record as one deterministic `Funzzy debug: key=value ...`
/// line. Field order is fixed; values containing whitespace or quotes are
/// quoted. Never emits ANSI.
pub fn render(record: &Record) -> String {
    let mut parts: Vec<(String, String)> = Vec::new();
    let mut push = |key: &'static str, value: Option<String>| {
        if let Some(value) = value {
            parts.push((key.to_owned(), value));
        }
    };

    push("seq", record.seq.map(|v| v.to_string()));
    push("batch", record.batch.map(|v| v.to_string()));
    push("source", record.source.map(str::to_owned));
    push("kind", record.kind.map(str::to_owned));
    push("path", record.path.clone());
    push("normalized", record.normalized.clone());
    push("decision", record.decision.map(str::to_owned));
    push("task", record.task.clone());
    push("change", record.change.clone());
    push("ignore", record.ignore.clone());
    push("rule_origin", record.rule_origin.clone());
    push("run", record.generation.map(|v| v.to_string()));
    push("policy", record.policy.map(str::to_owned));
    push("commands", record.commands.map(|v| v.to_string()));
    push(
        "command",
        record
            .command_position
            .map(|(index, total)| format!("{}/{}", index, total)),
    );
    push("state", record.state.map(str::to_owned));
    push("command", record.command.clone());
    push("duration", record.duration.clone());
    push("reason", record.reason.clone());
    push(
        "observed_after_run",
        record.observed_after_run.map(|v| v.to_string()),
    );
    push("repeats", record.repeats.map(|v| v.to_string()));
    push("hint", record.hint.clone());
    push("note", record.note.clone());

    format!(
        "Funzzy debug: {}",
        parts
            .iter()
            .map(|(key, value)| format!("{}={}", key, quote(value)))
            .collect::<Vec<_>>()
            .join(" ")
    )
}

/// Quotes a value when it contains whitespace or a double quote, escaping
/// embedded quotes. Deterministic: the same value always renders the same.
fn quote(value: &str) -> String {
    if value.is_empty()
        || value.contains(' ')
        || value.contains('"')
        || value.contains('\t')
        || value.contains('*')
        || value.contains('?')
        || value.contains('[')
        || value.contains('{')
    {
        format!("\"{}\"", value.replace('"', "\\\""))
    } else {
        value.to_owned()
    }
}

/// Feedback-loop policy: how far back repeats count and when to warn.
#[derive(Debug, Clone)]
pub struct LoopPolicy {
    /// Time window inside which repeats accumulate for one trigger chain.
    pub window: Duration,
    /// Warn once per key per window when the repeat count reaches this value.
    pub min_repeats: usize,
}

impl Default for LoopPolicy {
    fn default() -> Self {
        Self {
            window: Duration::from_secs(30),
            min_repeats: 3,
        }
    }
}

/// A bounded, deterministic feedback-loop warning. Observational only.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct LoopWarning {
    pub task: String,
    pub path: String,
    pub rule: String,
    pub repeats: u64,
    /// Generation related to the trigger chain, when known.
    pub generation: Option<u64>,
    pub hint: String,
}

/// Renders the loop warning body (the `stdout::warn` prefix is added by the
/// caller): `possible feedback loop task=... path=... change=... repeats=N
/// [run=G] hint=...`.
pub fn render_loop_warning(warning: &LoopWarning) -> String {
    let mut parts = vec![
        format!("task={}", quote(&warning.task)),
        format!("path={}", quote(&warning.path)),
        format!("change={}", quote(&warning.rule)),
        format!("repeats={}", warning.repeats),
    ];
    if let Some(generation) = warning.generation {
        parts.push(format!("run={}", generation));
    }
    parts.push(format!("hint={}", quote(&warning.hint)));
    format!("possible feedback loop {}", parts.join(" "))
}

/// Result of observing one trigger against the loop heuristic.
#[derive(Debug, Clone)]
pub struct LoopObservation {
    /// Repeat count for this task/path/rule within the window (1 = first).
    pub repeats: u64,
    /// Generation scheduled for this trigger, when known.
    pub generation: Option<u64>,
    /// A generation for the same task recorded within the window before this
    /// trigger; supports `observed_after_run` correlation. Never implies
    /// causation.
    pub observed_after_run: Option<u64>,
    /// Warning once per key per window when repeats cross the threshold.
    pub warning: Option<LoopWarning>,
}

#[derive(Debug, Clone, PartialEq, Eq, Hash)]
struct LoopKey {
    task: String,
    path: String,
    rule: String,
}

struct LoopEntry {
    count: u64,
    last_seen: Instant,
    warned: bool,
}

/// Bounded heuristic state for feedback-loop detection. Pure: `now` is
/// injected so behavior is deterministic and tests never depend on real
/// clocks. State is capped; expired keys are evicted lazily.
pub struct LoopDetector {
    policy: LoopPolicy,
    entries: HashMap<LoopKey, LoopEntry>,
    /// task -> (last seen, generation); correlates `observed_after_run`.
    task_recent: HashMap<String, (Instant, u64)>,
}

const MAX_KEYS: usize = 512;

impl LoopDetector {
    pub fn new(policy: LoopPolicy) -> Self {
        Self {
            policy,
            entries: HashMap::new(),
            task_recent: HashMap::new(),
        }
    }

    pub fn policy(&self) -> &LoopPolicy {
        &self.policy
    }

    /// Observes one trigger for `task`/`path`/`rule` at time `now` with an
    /// optional scheduled generation. Returns repeat/observed-after-run
    /// correlation and a warning when the threshold is crossed for the first
    /// time in this window. Never mutates scheduling state.
    pub fn observe(
        &mut self,
        now: Instant,
        task: &str,
        path: &str,
        rule: &str,
        generation: Option<u64>,
    ) -> LoopObservation {
        self.prune(now);

        let key = LoopKey {
            task: task.to_owned(),
            path: path.to_owned(),
            rule: rule.to_owned(),
        };

        // A generation for the same task seen within the window before this
        // trigger; the previous trigger's run, never the current one.
        let observed_after_run = self
            .task_recent
            .get(task)
            .filter(|(seen, _)| now.duration_since(*seen) <= self.policy.window)
            .map(|(_, run)| *run);

        let (repeats, warning) = {
            let entry = self.entries.entry(key).or_insert_with(|| LoopEntry {
                count: 0,
                last_seen: now,
                warned: false,
            });
            if now.duration_since(entry.last_seen) > self.policy.window {
                // Window expired: start a fresh chain for this key.
                *entry = LoopEntry {
                    count: 0,
                    last_seen: now,
                    warned: false,
                };
            }
            entry.count += 1;
            entry.last_seen = now;

            if let Some(run) = generation {
                self.task_recent.insert(task.to_owned(), (now, run));
            }

            let warning = if entry.count >= self.policy.min_repeats as u64 && !entry.warned {
                entry.warned = true;
                Some(LoopWarning {
                    task: task.to_owned(),
                    path: path.to_owned(),
                    rule: rule.to_owned(),
                    repeats: entry.count,
                    generation: generation.or(observed_after_run),
                    hint: hint_for(path),
                })
            } else {
                None
            };
            (entry.count, warning)
        };
        self.cap();

        LoopObservation {
            repeats,
            generation,
            observed_after_run,
            warning,
        }
    }

    /// Removes expired keys and keeps the map bounded. Deterministic eviction
    /// picks the oldest `last_seen` when the cap is exceeded.
    fn prune(&mut self, now: Instant) {
        self.entries
            .retain(|_, entry| now.duration_since(entry.last_seen) <= self.policy.window);
        self.task_recent
            .retain(|_, (seen, _)| now.duration_since(*seen) <= self.policy.window);
        self.cap();
    }

    /// Bounds both maps to [`MAX_KEYS`] entries by evicting the oldest.
    fn cap(&mut self) {
        while self.entries.len() > MAX_KEYS {
            let oldest = self
                .entries
                .iter()
                .min_by_key(|(_, entry)| entry.last_seen)
                .map(|(key, _)| key.clone());
            let Some(oldest) = oldest else { break };
            self.entries.remove(&oldest);
        }
        while self.task_recent.len() > MAX_KEYS {
            let oldest = self
                .task_recent
                .iter()
                .min_by_key(|(_, (seen, _))| *seen)
                .map(|(task, _)| task.clone());
            let Some(oldest) = oldest else { break };
            self.task_recent.remove(&oldest);
        }
    }
}

/// Suggest an ignore direction for a repeated path: the containing directory
/// as a glob, e.g. `generated/api.rs` -> `consider ignoring generated/**`.
fn hint_for(path: &str) -> String {
    match path.rfind('/') {
        Some(index) if index > 0 => format!("consider ignoring {}/**", &path[..index]),
        _ => format!("consider ignoring {}", path),
    }
}

/// Process-wide diagnostic sink, mirroring `logging.rs`: initialized once at
/// the composition root; no-op when verbose mode is disabled.
struct Sink {
    seq: AtomicU64,
    detector: Mutex<LoopDetector>,
}

static STATE: LazyLock<Mutex<Option<Sink>>> = LazyLock::new(|| Mutex::new(None));

/// Enables or disables diagnostics for the process. Called once from the
/// composition root with the verbose flag.
pub fn init(enabled: bool) {
    let mut state = STATE.lock().unwrap();
    *state = if enabled {
        Some(Sink {
            seq: AtomicU64::new(0),
            detector: Mutex::new(LoopDetector::new(LoopPolicy::default())),
        })
    } else {
        None
    };
}

pub fn enabled() -> bool {
    STATE.lock().map(|state| state.is_some()).unwrap_or(false)
}

/// Emits one deterministic debug record to the terminal and log file (same
/// semantics; the log strips ANSI, and records never contain any). No-op
/// when verbose mode is disabled.
pub fn debug(record: &Record) {
    let mut state = STATE.lock().unwrap();
    let Some(sink) = state.as_mut() else {
        return;
    };
    let seq = sink.seq.fetch_add(1, Ordering::Relaxed) + 1;
    let mut record = record.clone();
    record.seq = Some(seq);
    let rendered = render(&record);
    println!("{}", rendered);
    crate::logging::log_line(&rendered);
}

/// Emits a feedback-loop warning through the standard warning channel. No-op
/// when verbose mode is disabled.
pub fn warn_loop(warning: &LoopWarning) {
    if !enabled() {
        return;
    }
    crate::stdout::warn(&render_loop_warning(warning));
}

/// Observes one trigger through the shared loop detector. Returns `None`
/// (and records nothing) when verbose mode is disabled, so callers can skip
/// correlated `observed_after_run` records.
pub fn observe(
    task: &str,
    path: &str,
    rule: &str,
    generation: Option<u64>,
) -> Option<LoopObservation> {
    let mut state = STATE.lock().unwrap();
    let sink = state.as_mut()?;
    let mut detector = sink.detector.lock().unwrap();
    Some(detector.observe(Instant::now(), task, path, rule, generation))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn record() -> Record {
        Record {
            batch: Some(17),
            source: Some("filesystem"),
            kind: Some("any"),
            path: Some("generated/api.rs".to_owned()),
            normalized: Some("generated/api.rs".to_owned()),
            ..Default::default()
        }
    }

    #[test]
    fn event_record_renders_stable_vocabulary() {
        assert_eq!(
            render(&record()),
            "Funzzy debug: batch=17 source=filesystem kind=any path=generated/api.rs normalized=generated/api.rs"
        );
    }

    #[test]
    fn matched_decision_record_renders_task_rule_and_origin() {
        let record = Record {
            batch: Some(17),
            decision: Some("matched"),
            task: Some("generate API".to_owned()),
            change: Some("**/*.rs".to_owned()),
            rule_origin: Some("task".to_owned()),
            ..Default::default()
        };
        assert_eq!(
            render(&record),
            "Funzzy debug: batch=17 decision=matched task=\"generate API\" change=\"**/*.rs\" rule_origin=task"
        );
    }

    #[test]
    fn scheduled_run_record_renders_generation_policy_and_command_count() {
        let record = Record {
            batch: Some(17),
            generation: Some(42),
            policy: Some("restart"),
            commands: Some(1),
            ..Default::default()
        };
        assert_eq!(
            render(&record),
            "Funzzy debug: batch=17 run=42 policy=restart commands=1"
        );
    }

    #[test]
    fn command_started_record_renders_position_state_and_command() {
        let record = Record {
            generation: Some(42),
            command_position: Some((1, 1)),
            state: Some("started"),
            command: Some("make generate".to_owned()),
            ..Default::default()
        };
        assert_eq!(
            render(&record),
            "Funzzy debug: run=42 command=1/1 state=started command=\"make generate\""
        );
    }

    #[test]
    fn finished_record_renders_state_and_duration() {
        let record = Record {
            generation: Some(42),
            state: Some("passed"),
            duration: Some("0.842s".to_owned()),
            ..Default::default()
        };
        assert_eq!(
            render(&record),
            "Funzzy debug: run=42 state=passed duration=0.842s"
        );
    }

    #[test]
    fn cancelled_record_renders_reason() {
        let record = Record {
            generation: Some(42),
            state: Some("cancelled"),
            reason: Some("replaced".to_owned()),
            ..Default::default()
        };
        assert_eq!(
            render(&record),
            "Funzzy debug: run=42 state=cancelled reason=replaced"
        );
    }

    #[test]
    fn observed_after_run_record_renders_correlation() {
        let record = Record {
            batch: Some(18),
            source: Some("filesystem"),
            kind: Some("modify"),
            path: Some("generated/api.rs".to_owned()),
            observed_after_run: Some(42),
            ..Default::default()
        };
        assert_eq!(
            render(&record),
            "Funzzy debug: batch=18 source=filesystem kind=modify path=generated/api.rs observed_after_run=42"
        );
    }

    #[test]
    fn seq_renders_first_when_present() {
        let record = Record {
            seq: Some(7),
            decision: Some("startup"),
            ..Default::default()
        };
        assert_eq!(render(&record), "Funzzy debug: seq=7 decision=startup");
    }

    #[test]
    fn empty_record_renders_bare_prefix() {
        assert_eq!(render(&Record::default()), "Funzzy debug: ");
    }

    #[test]
    fn values_with_spaces_are_quoted_and_embedded_quotes_escaped() {
        let record = Record {
            task: Some("say \"hi\" now".to_owned()),
            ..Default::default()
        };
        assert_eq!(render(&record), "Funzzy debug: task=\"say \\\"hi\\\" now\"");
    }

    #[test]
    fn render_never_contains_ansi() {
        let rendered = render(&record());
        assert!(
            !rendered.contains('\u{1b}'),
            "diagnostics must never carry ANSI: {:?}",
            rendered
        );
    }

    #[test]
    fn warning_renders_repeat_count_and_hint_with_quoted_values() {
        let warning = LoopWarning {
            task: "generate API".to_owned(),
            path: "generated/api.rs".to_owned(),
            rule: "**/*.rs".to_owned(),
            repeats: 3,
            generation: Some(42),
            hint: "consider ignoring generated/**".to_owned(),
        };
        assert_eq!(
            render_loop_warning(&warning),
            "possible feedback loop task=\"generate API\" path=generated/api.rs change=\"**/*.rs\" repeats=3 run=42 hint=\"consider ignoring generated/**\""
        );
    }

    fn now() -> Instant {
        Instant::now()
    }

    #[test]
    fn loop_detector_first_trigger_reports_repeat_one_and_no_warning() {
        let mut detector = LoopDetector::new(LoopPolicy::default());
        let observation = detector.observe(
            now(),
            "generate API",
            "generated/api.rs",
            "**/*.rs",
            Some(42),
        );
        assert_eq!(observation.repeats, 1);
        assert_eq!(observation.generation, Some(42));
        assert!(observation.observed_after_run.is_none());
        assert!(observation.warning.is_none());
    }

    #[test]
    fn loop_detector_warns_once_when_repeat_threshold_is_crossed() {
        let mut detector = LoopDetector::new(LoopPolicy {
            window: Duration::from_secs(30),
            min_repeats: 3,
        });
        let base = now();
        let mut at = base;
        let step = Duration::from_millis(10);

        assert!(detector
            .observe(at, "t", "p.rs", "r", Some(1))
            .warning
            .is_none());
        at += step;
        assert!(detector
            .observe(at, "t", "p.rs", "r", Some(2))
            .warning
            .is_none());
        at += step;
        let third = detector.observe(at, "t", "p.rs", "r", Some(3));
        let warning = third.warning.expect("third repeat must warn");
        assert_eq!(warning.repeats, 3);
        assert_eq!(warning.task, "t");
        assert_eq!(warning.path, "p.rs");
        assert_eq!(warning.rule, "r");
        assert_eq!(warning.generation, Some(3));
        assert_eq!(warning.hint, "consider ignoring p.rs");
        at += step;
        // A further repeat within the window must NOT warn again (bounded).
        assert!(
            detector
                .observe(at, "t", "p.rs", "r", Some(4))
                .warning
                .is_none(),
            "warnings must be emitted once per window per key"
        );
    }

    #[test]
    fn loop_detector_ignores_unrelated_rapid_events() {
        let mut detector = LoopDetector::new(LoopPolicy {
            window: Duration::from_secs(30),
            min_repeats: 3,
        });
        let base = now();
        // Different paths/tasks never share a chain, so rapid unrelated events
        // must never produce a warning.
        for i in 0..20 {
            let at = base + Duration::from_millis(i);
            let observation = detector.observe(
                at,
                &format!("task-{}", i),
                &format!("file-{}.rs", i),
                "**",
                None,
            );
            assert!(
                observation.warning.is_none(),
                "unrelated rapid events must not warn: {:?}",
                observation
            );
            assert_eq!(observation.repeats, 1);
        }
    }

    #[test]
    fn loop_detector_resets_after_window_expiry() {
        let mut detector = LoopDetector::new(LoopPolicy {
            window: Duration::from_secs(10),
            min_repeats: 2,
        });
        let base = now();
        assert!(detector
            .observe(base, "t", "p.rs", "r", None)
            .warning
            .is_none());
        let warning = detector
            .observe(base + Duration::from_secs(1), "t", "p.rs", "r", None)
            .warning
            .expect("second repeat within window must warn");
        assert_eq!(warning.repeats, 2);

        // Window expires: the chain restarts and the next repeat is a fresh
        // first occurrence, so no immediate warning.
        let observation = detector.observe(base + Duration::from_secs(12), "t", "p.rs", "r", None);
        assert_eq!(observation.repeats, 1);
        assert!(observation.warning.is_none());
    }

    #[test]
    fn loop_detector_correlates_observed_after_run_for_same_task() {
        let mut detector = LoopDetector::new(LoopPolicy::default());
        let base = now();

        // First trigger schedules generation 42 for task "t".
        let first = detector.observe(base, "t", "a.rs", "**", Some(42));
        assert!(first.observed_after_run.is_none());

        // A later trigger for the same task (different path) observes the
        // previous generation, never claiming causation.
        let second = detector.observe(base + Duration::from_secs(1), "t", "b.rs", "**", Some(43));
        assert_eq!(second.observed_after_run, Some(42));

        // A different task never observes "t"'s generation.
        let other = detector.observe(base + Duration::from_secs(2), "u", "c.rs", "**", Some(44));
        assert!(other.observed_after_run.is_none());
    }

    #[test]
    fn loop_detector_state_stays_bounded() {
        let mut detector = LoopDetector::new(LoopPolicy::default());
        let base = now();
        // Exceed the cap with distinct keys; state must stay bounded and the
        // detector must remain usable.
        for i in 0..(MAX_KEYS + 100) {
            let _ = detector.observe(
                base + Duration::from_millis(i as u64),
                &format!("task-{}", i),
                &format!("file-{}.rs", i),
                "**",
                None,
            );
        }
        assert!(detector.entries.len() <= MAX_KEYS);
        assert!(detector.task_recent.len() <= MAX_KEYS);
        // A fresh key still works after eviction pressure.
        let observation =
            detector.observe(base + Duration::from_secs(1), "fresh", "new.rs", "**", None);
        assert_eq!(observation.repeats, 1);
    }

    #[test]
    fn hint_uses_containing_directory_for_nested_paths() {
        assert_eq!(
            hint_for("generated/api.rs"),
            "consider ignoring generated/**"
        );
        assert_eq!(
            hint_for("src/generated/api.rs"),
            "consider ignoring src/generated/**"
        );
        assert_eq!(hint_for("single.rs"), "consider ignoring single.rs");
    }

    #[test]
    fn disabled_sink_debug_is_a_noop() {
        init(false);
        assert!(!enabled());
        // Must not panic and must not print: no assertion beyond returning.
        debug(&record());
        assert!(observe("t", "p.rs", "**", None).is_none());
    }
}
