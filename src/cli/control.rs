use crate::cli::Command;
use crate::config;
use crate::control_client::{
    AwaitMode, AwaitSnapshot, CancelSnapshot, CapabilitiesSnapshot, ControlClient,
    ControlClientError, EmitSnapshot, OutputSnapshot, StatusSnapshot, TargetSnapshot,
};
use crate::duration_history::RunEstimate;
use crate::errors::FzzError;
use std::path::{Path, PathBuf};
use std::time::Duration;

/// The action a `fzz control` invocation performs. Rendering stays here;
/// transport and protocol live in `control_client`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ControlAction {
    Status,
    List,
    Capabilities,
    Run {
        target: String,
        /// Await the exact scheduled generation before returning (TASK-0044).
        wait: bool,
        /// Required with `--wait`; bounds the server-side await.
        timeout: Option<Duration>,
    },
    Emit {
        path: String,
        /// Await the scheduled generation before returning, when one exists.
        wait: bool,
        /// Required with `--wait`; bounds the server-side await.
        timeout: Option<Duration>,
    },
    Await {
        after: Option<u64>,
        generation: Option<u64>,
        timeout: Duration,
    },
    Cancel {
        generation: u64,
        /// Await the exact generation to terminal after cancelling.
        wait: bool,
        /// Required with `--wait`; bounds the server-side await.
        timeout: Option<Duration>,
    },
    Output {
        generation: u64,
        task: Option<String>,
        stream: Option<String>,
        tail: Option<u64>,
        full: bool,
    },
}

/// Parses a CLI duration bound: `<number>` (seconds), or `<number>ms`/`s`/`m`.
/// Zero is rejected; waits are always positive and bounded.
pub fn parse_duration(input: &str) -> Result<Duration, String> {
    let input = input.trim();
    let (digits, multiplier) = if let Some(stripped) = input.strip_suffix("ms") {
        (stripped, 1u64)
    } else if let Some(stripped) = input.strip_suffix('s') {
        (stripped, 1_000u64)
    } else if let Some(stripped) = input.strip_suffix('m') {
        (stripped, 60_000u64)
    } else {
        (input, 1_000u64)
    };
    let value: u64 = digits.trim().parse().map_err(|_| {
        format!(
            "invalid duration '{}': expected <number> with optional ms/s/m suffix (bare number = seconds)",
            input
        )
    })?;
    if value == 0 {
        return Err(format!("invalid duration '{}': must be positive", input));
    }
    let millis = value
        .checked_mul(multiplier)
        .ok_or_else(|| format!("invalid duration '{}': bound is too large", input))?;
    Ok(Duration::from_millis(millis))
}

/// `fzz control` client command group (TASK-0021/0022). Consumes the
/// existing `status`, `targets`, `run`, and `emit` methods; `--wait`/
/// `--timeout` and `await` land in TASK-0044.
pub struct ControlCommand {
    action: ControlAction,
    /// `control --socket <PATH>`: highest-precedence socket override.
    socket_override: Option<String>,
    /// Global `--control-socket <PATH>`.
    global_socket: Option<String>,
    /// `-c/--config <FILE>` used to locate `on.socket`.
    config: Option<String>,
}

impl ControlCommand {
    pub fn new(
        action: ControlAction,
        socket_override: Option<String>,
        global_socket: Option<String>,
        config: Option<String>,
    ) -> Self {
        Self {
            action,
            socket_override,
            global_socket,
            config,
        }
    }

    /// Socket path resolution contract: explicit `--socket` wins, then
    /// global `--control-socket`, then `on.socket` from the selected config
    /// file (default `.watch.yaml`/`.watch.yml`), else an actionable error.
    /// Relative `on.socket` paths resolve against the invoking directory,
    /// matching the watcher's own binding behavior.
    fn resolve_socket(&self) -> Result<PathBuf, String> {
        if let Some(path) = self
            .socket_override
            .as_ref()
            .filter(|value| !value.trim().is_empty())
        {
            return Ok(PathBuf::from(path));
        }
        if let Some(path) = self
            .global_socket
            .as_ref()
            .filter(|value| !value.trim().is_empty())
        {
            return Ok(PathBuf::from(path));
        }
        for candidate in self.config_candidates() {
            if !Path::new(&candidate).exists() {
                return Err(format!("config file not found: {}", candidate));
            }
            if let Some(socket) = config::control_socket_from_file(&candidate)
                .map_err(|err| format!("invalid control socket config in {}: {}", candidate, err))?
            {
                return Ok(PathBuf::from(socket));
            }
        }
        Err("no control socket configured: provide `--socket <PATH>`, `--control-socket <PATH>`, or set `on.socket` in the config file".to_string())
    }

    fn config_candidates(&self) -> Vec<String> {
        match &self.config {
            Some(path) => vec![path.clone()],
            None => {
                let mut candidates = Vec::new();
                if Path::new(crate::cli::watch::DEFAULT_FILENAME).exists() {
                    candidates.push(crate::cli::watch::DEFAULT_FILENAME.to_string());
                }
                let alternative = crate::cli::watch::DEFAULT_FILENAME.replace("yaml", "yml");
                if alternative != crate::cli::watch::DEFAULT_FILENAME
                    && Path::new(&alternative).exists()
                {
                    candidates.push(alternative);
                }
                candidates
            }
        }
    }
}

impl Command for ControlCommand {
    fn execute(&self) -> Result<(), FzzError> {
        let path = self.resolve_socket().map_err(FzzError::GenericError)?;
        let mut client =
            ControlClient::connect(&path).map_err(|err| FzzError::GenericError(err.to_string()))?;
        match &self.action {
            ControlAction::Status => {
                let status = client
                    .status()
                    .map_err(|err| FzzError::GenericError(err.to_string()))?;
                print!("{}", render_status(&status));
            }
            ControlAction::List => {
                let targets = client
                    .targets()
                    .map_err(|err| FzzError::GenericError(err.to_string()))?;
                print!("{}", render_targets(&targets));
            }
            ControlAction::Capabilities => {
                let capabilities = client
                    .capabilities()
                    .map_err(|err| FzzError::GenericError(err.to_string()))?;
                print!("{}", render_capabilities(&capabilities));
            }
            ControlAction::Run {
                target,
                wait,
                timeout,
            } => {
                let generation = client
                    .run(target)
                    .map_err(|err| FzzError::GenericError(err.to_string()))?;
                print!("{}", render_run(generation));
                if *wait {
                    return self.finish_await(
                        &mut client,
                        &path,
                        AwaitMode::Exact(generation),
                        timeout
                            .as_ref()
                            .expect("--wait requires --timeout (validated by clap)"),
                    );
                }
            }
            ControlAction::Emit {
                path: emit_path,
                wait,
                timeout,
            } => {
                let emit = client
                    .emit(emit_path)
                    .map_err(|err| FzzError::GenericError(err.to_string()))?;
                print!("{}", render_emit(&emit));
                if *wait {
                    if let Some(generation) = emit.run_id {
                        return self.finish_await(
                            &mut client,
                            &path,
                            AwaitMode::Exact(generation),
                            timeout
                                .as_ref()
                                .expect("--wait requires --timeout (validated by clap)"),
                        );
                    }
                    // No generation was scheduled: the explicit no-op outcome
                    // is the observation (exit 0).
                }
            }
            ControlAction::Await {
                after,
                generation,
                timeout,
            } => {
                let mode = match (after, generation) {
                    (Some(after), None) => AwaitMode::After(*after),
                    (None, Some(generation)) => AwaitMode::Exact(*generation),
                    _ => unreachable!("validated mutually exclusive await modes"),
                };
                return self.finish_await(&mut client, &path, mode, timeout);
            }
            ControlAction::Cancel {
                generation,
                wait,
                timeout,
            } => {
                // Negotiate the instance token so a stale request formed
                // against a different watcher process is a safe no-op.
                let token = client.capabilities().ok().map(|caps| caps.token);
                let cancel = client
                    .cancel(*generation, token.as_deref())
                    .map_err(|err| FzzError::GenericError(err.to_string()))?;
                print!("{}", render_cancel(&cancel));
                if *wait {
                    return self.finish_await(
                        &mut client,
                        &path,
                        AwaitMode::Exact(*generation),
                        timeout
                            .as_ref()
                            .expect("--wait requires --timeout (validated by clap)"),
                    );
                }
            }
            ControlAction::Output {
                generation,
                task,
                stream,
                tail,
                full,
            } => {
                let retrieved = client
                    .output(
                        *generation,
                        task.as_deref(),
                        stream.as_deref(),
                        *tail,
                        *full,
                    )
                    .map_err(|err| FzzError::GenericError(err.to_string()))?;
                print!("{}", render_output(&retrieved));
            }
        }
        Ok(())
    }
}

impl ControlCommand {
    /// Runs the await and maps the outcome to the AXI exit-code contract
    /// (contract §8): passed/cancelled -> 0; failed/superseded/timeout -> 1;
    /// disconnected/restarted -> 1 with the reason surfaced. The observation
    /// always renders first; the trailing error line carries the exit code.
    fn finish_await(
        &self,
        client: &mut ControlClient,
        path: &Path,
        mode: AwaitMode,
        timeout: &Duration,
    ) -> Result<(), FzzError> {
        // Capture the instance token before waiting so a transport failure
        // can distinguish restart (token changed) from disconnect.
        let token_before = client
            .capabilities()
            .ok()
            .map(|capabilities| capabilities.token);
        match client.await_generation(mode, timeout.as_millis() as u64) {
            Ok(observation) => {
                let rendered = render_await(&observation);
                print!("{}", rendered);
                match observation.terminal_reason.as_str() {
                    "passed" | "cancelled" => Ok(()),
                    reason => Err(FzzError::GenericError(format!(
                        "await: generation {} {}",
                        observation.snapshot.generation, reason
                    ))),
                }
            }
            Err(
                ControlClientError::Unavailable { .. }
                | ControlClientError::Io(_)
                | ControlClientError::Timeout
                | ControlClientError::Disconnected,
            ) => {
                // Re-negotiate capabilities to tell restart from disconnect
                // (contract §5).
                // Re-negotiation reconnects fresh and retries briefly, so a
                // restarting watcher that rebinds the same socket path within
                // the window is detected as `restarted` instead of
                // `disconnected`. The dead connection can never be reused.
                let restarted = if let Some(before) = &token_before {
                    let mut renegotiated = false;
                    for _ in 0..20 {
                        match ControlClient::connect(path) {
                            Ok(mut fresh) => match fresh.capabilities() {
                                Ok(caps) => {
                                    renegotiated = caps.token != *before;
                                    break;
                                }
                                Err(_) => {}
                            },
                            Err(_) => {}
                        }
                        std::thread::sleep(Duration::from_millis(250));
                    }
                    renegotiated
                } else {
                    false
                };
                let reason = if restarted {
                    "restarted"
                } else {
                    "disconnected"
                };
                Err(FzzError::GenericError(format!(
                    "await: watcher {} while waiting",
                    reason
                )))
            }
            Err(err) => Err(FzzError::GenericError(err.to_string())),
        }
    }
}

/// Compact deterministic `status` rendering: generation, state, trigger,
/// duration, commands, and failures. Raw command output stays on the
/// watcher side; this is correlation data only.
pub fn render_status(status: &StatusSnapshot) -> String {
    let mut output = String::new();
    output.push_str(&format!("generation: {}\n", status.generation));
    output.push_str(&format!("state: {}\n", status.state));
    if let Some(trigger) = &status.trigger {
        output.push_str(&format!("trigger: {}\n", trigger));
    }
    if let Some(duration_ms) = status.duration_ms {
        output.push_str(&format!("duration_ms: {}\n", duration_ms));
    }
    if status.commands.is_empty() {
        output.push_str("commands: (none)\n");
    } else {
        output.push_str("commands:\n");
        for command in &status.commands {
            output.push_str(&format!("  - {}\n", command));
        }
    }
    if status.failures.is_empty() {
        output.push_str("failures: (none)\n");
    } else {
        output.push_str("failures:\n");
        for failure in &status.failures {
            output.push_str(&format!("  - {}\n", failure));
        }
    }
    output
}

/// Compact deterministic `capabilities` rendering: protocol facts a client
/// gates on, not dynamic watcher state.
pub fn render_capabilities(caps: &CapabilitiesSnapshot) -> String {
    let mut output = String::new();
    output.push_str(&format!("protocol version: {}\n", caps.protocol_version));
    output.push_str(&format!("schema version: {}\n", caps.schema_version));
    output.push_str(&format!("watcher version: {}\n", caps.watcher_version));
    output.push_str(&format!("instance token: {}\n", caps.token));
    output.push_str("methods:\n");
    for method in &caps.methods {
        output.push_str(&format!("  - {}\n", method));
    }
    output.push_str("output formats:\n");
    for format in &caps.output_formats {
        output.push_str(&format!("  - {}\n", format));
    }
    output.push_str("optional fields:\n");
    for field in &caps.optional_fields {
        output.push_str(&format!("  - {}\n", field));
    }
    output.push_str("limits:\n");
    output.push_str(&format!(
        "  output retention bytes: {}\n",
        caps.limits.output_retention_bytes
    ));
    output.push_str(&format!(
        "  max response bytes: {}\n",
        caps.limits.max_response_bytes
    ));
    output.push_str(&format!(
        "  max evidence lines: {}\n",
        caps.limits.max_evidence_lines
    ));
    if caps.features.duration_estimates {
        output.push_str(&format!(
            "  estimate max samples: {}\n",
            caps.limits.estimate_max_samples
        ));
        output.push_str(&format!(
            "  estimate floor ms: {}\n",
            caps.limits.estimate_floor_ms
        ));
        output.push_str(&format!(
            "  estimate cap ms: {}\n",
            caps.limits.estimate_cap_ms
        ));
    }
    output.push_str("features:\n");
    output.push_str(&format!("  atomic await: {}\n", caps.features.atomic_await));
    output.push_str(&format!("  subscription: {}\n", caps.features.subscription));
    output.push_str(&format!(
        "  correlated snapshots: {}\n",
        caps.features.correlated_snapshots
    ));
    output.push_str(&format!(
        "  output retrieval: {}\n",
        caps.features.output_retrieval
    ));
    output.push_str(&format!("  pending work: {}\n", caps.features.pending_work));
    output.push_str(&format!(
        "  duration estimates: {}\n",
        caps.features.duration_estimates
    ));
    output
}

/// Compact deterministic `targets` rendering from the running watcher.
pub fn render_targets(targets: &[TargetSnapshot]) -> String {
    if targets.is_empty() {
        return "targets: (none)\n".to_string();
    }
    let mut output = format!("targets ({}):\n", targets.len());
    for target in targets {
        output.push_str(&format!("  - {}\n", target.name));
        for command in &target.commands {
            output.push_str(&format!("      {}\n", command));
        }
        if let Some(estimate) = &target.estimate {
            output.push_str(&format!("      {}\n", render_estimate(estimate)));
        }
    }
    output
}

/// Deterministic compact duration-estimate rendering shared by targets and
/// snapshots (TASK-0055): the same domain estimate, one stable format.
pub fn render_estimate(estimate: &RunEstimate) -> String {
    format!(
        "estimate: typical={} upper={} timeout={} confidence={} n={} source={}",
        format_duration(estimate.typical_ms),
        format_duration(estimate.upper_ms),
        format_duration(estimate.recommended_timeout_ms),
        confidence_label(estimate.confidence),
        estimate.samples,
        source_label(estimate.source),
    )
}

/// Deterministic human duration: `42ms`, `1.5s`, `2m`, `3h` (no sub-second
/// noise, no locale-dependent formatting).
pub fn format_duration(ms: u64) -> String {
    if ms < 1_000 {
        format!("{}ms", ms)
    } else if ms < 60_000 {
        let seconds = ms as f64 / 1_000.0;
        format!("{:.1}s", seconds)
    } else if ms < 3_600_000 {
        format!("{}m{}", ms / 60_000, (ms % 60_000) / 1_000)
    } else {
        format!("{}h{}m", ms / 3_600_000, (ms % 3_600_000) / 60_000)
    }
}

fn confidence_label(confidence: crate::duration_history::EstimateConfidence) -> &'static str {
    match confidence {
        crate::duration_history::EstimateConfidence::None => "none",
        crate::duration_history::EstimateConfidence::Low => "low",
        crate::duration_history::EstimateConfidence::Medium => "medium",
        crate::duration_history::EstimateConfidence::High => "high",
    }
}

fn source_label(source: crate::duration_history::EstimateSource) -> &'static str {
    match source {
        crate::duration_history::EstimateSource::Measured => "measured",
        crate::duration_history::EstimateSource::Configured => "configured",
    }
}

/// Scheduled-generation identity returned by `control run TARGET`.
pub fn render_run(generation: u64) -> String {
    format!("scheduled generation: {}\n", generation)
}

/// One consistent await observation (contract §4): terminal reason, freshness,
/// pending debounce state, and the snapshot it belongs to.
pub fn render_await(observation: &AwaitSnapshot) -> String {
    let mut output = format!("terminal reason: {}\n", observation.terminal_reason);
    output.push_str(&format!("freshness: {}\n", observation.freshness));
    output.push_str(&format!(
        "latest generation: {}\n",
        observation.latest_generation
    ));
    if let Some(batch) = observation.latest_batch {
        output.push_str(&format!("latest batch: {}\n", batch));
    }
    output.push_str(&format!(
        "pending debounce: {}\n",
        observation.pending_work.debounce_active
    ));
    output.push_str(&format!(
        "queued batches: {}\n",
        observation.pending_work.queued_batches
    ));
    output.push_str("snapshot:\n");
    output.push_str(&render_status(&observation.snapshot));
    if let Some(evidence) = &observation.failure_evidence {
        output.push_str("failure evidence:\n");
        output.push_str(&format!("  truncated: {}\n", evidence.truncated));
        output.push_str(&format!(
            "  observed_bytes: {}\n",
            evidence.total_observed_bytes
        ));
        output.push_str(&format!("  retained_bytes: {}\n", evidence.retained_bytes));
        output.push_str(&format!("  retrieve: {}\n", evidence.retrieve));
        output.push_str("  excerpt:\n");
        for line in evidence.excerpt.lines() {
            output.push_str(&format!("    {}\n", line));
        }
    }
    output
}

/// Bounded retrieval rendering (contract §6): per task and stream, the
/// content plus bounds metadata. Command output may contain secrets — the
/// socket permission (0600) is the security boundary.
pub fn render_output(output: &OutputSnapshot) -> String {
    let mut rendered = format!("output: generation {}\n", output.generation);
    if output.tasks.is_empty() {
        rendered.push_str("tasks: (none)\n");
        return rendered;
    }
    for task in &output.tasks {
        rendered.push_str(&format!("task: {}\n", task.id));
        for (name, stream) in [("stdout", &task.stdout), ("stderr", &task.stderr)] {
            let Some(stream) = stream else { continue };
            rendered.push_str(&format!(
                "  {name}: {} lines, retained {} bytes, observed {} bytes{}:\n",
                stream.lines,
                stream.retained_bytes,
                stream.observed_bytes,
                if stream.truncated { " (truncated)" } else { "" }
            ));
            rendered.push_str("  ---\n");
            for line in stream.content.lines() {
                rendered.push_str(&format!("  {}\n", line));
            }
            if !stream.content.is_empty() && !stream.content.ends_with('\n') {
                rendered.push_str("\n");
            }
        }
    }
    rendered
}

/// Compact deterministic `cancel` rendering: generation plus the compare-and-
/// cancel outcome (`cancelled` or `not-running`).
pub fn render_cancel(cancel: &CancelSnapshot) -> String {
    format!(
        "generation: {}\noutcome: {}\n",
        cancel.generation,
        if cancel.cancelled {
            "cancelled"
        } else {
            "not-running"
        }
    )
}

/// Compact deterministic `emit` rendering: outcome, matched tasks, and the
/// scheduled generation when one was produced.
pub fn render_emit(emit: &EmitSnapshot) -> String {
    let mut output = format!("outcome: {}\n", emit.outcome);
    if emit.matched.is_empty() {
        output.push_str("matched: (none)\n");
    } else {
        output.push_str(&format!("matched ({}):\n", emit.matched.len()));
        for name in &emit.matched {
            output.push_str(&format!("  - {}\n", name));
        }
    }
    if let Some(run_id) = emit.run_id {
        output.push_str(&format!("scheduled generation: {}\n", run_id));
    }
    output
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn render_status_covers_all_legacy_fields() {
        let status = StatusSnapshot {
            generation: 4,
            state: "failed".to_string(),
            trigger: Some("src/main.rs".to_string()),
            commands: vec!["cargo test".to_string()],
            duration_ms: Some(42),
            failures: vec!["invalid concurrency value".to_string()],
        };
        let rendered = render_status(&status);
        assert!(rendered.contains("generation: 4"));
        assert!(rendered.contains("state: failed"));
        assert!(rendered.contains("trigger: src/main.rs"));
        assert!(rendered.contains("duration_ms: 42"));
        assert!(rendered.contains("  - cargo test"));
        assert!(rendered.contains("  - invalid concurrency value"));
        assert!(!rendered.contains("(none)"));
    }

    #[test]
    fn render_status_marks_absent_optional_fields() {
        let status = StatusSnapshot {
            generation: 0,
            state: "idle".to_string(),
            trigger: None,
            commands: vec![],
            duration_ms: None,
            failures: vec![],
        };
        let rendered = render_status(&status);
        assert!(rendered.contains("state: idle"));
        assert!(!rendered.contains("trigger:"));
        assert!(!rendered.contains("duration_ms:"));
        assert!(rendered.contains("commands: (none)"));
        assert!(rendered.contains("failures: (none)"));
    }

    #[test]
    fn render_await_includes_reason_freshness_and_snapshot() {
        let observation = AwaitSnapshot {
            terminal_reason: "passed".to_string(),
            latest_generation: 7,
            latest_batch: Some(3),
            pending_work: crate::control_client::PendingWorkSnapshot {
                debounce_active: false,
                queued_batches: 0,
            },
            freshness: "current".to_string(),
            snapshot: StatusSnapshot {
                generation: 7,
                state: "passed".to_string(),
                trigger: Some("src/main.rs".to_string()),
                commands: vec!["cargo test".to_string()],
                duration_ms: Some(42),
                failures: vec![],
            },
            failure_evidence: None,
        };
        let rendered = render_await(&observation);
        assert!(rendered.contains("terminal reason: passed"));
        assert!(rendered.contains("freshness: current"));
        assert!(rendered.contains("latest generation: 7"));
        assert!(rendered.contains("latest batch: 3"));
        assert!(rendered.contains("pending debounce: false"));
        assert!(rendered.contains("queued batches: 0"));
        assert!(rendered.contains("snapshot:"));
        assert!(rendered.contains("generation: 7"));
        assert!(rendered.contains("state: passed"));
    }

    #[test]
    fn render_output_shows_tasks_streams_and_bounds() {
        use crate::control_client::{OutputSnapshot, RetrievedTaskSnapshot, StreamSnapshot};
        let output = OutputSnapshot {
            generation: 7,
            tasks: vec![RetrievedTaskSnapshot {
                id: "my tests".to_string(),
                stdout: Some(StreamSnapshot {
                    content: "line one\nline two\n".to_string(),
                    lines: 2,
                    retained_bytes: 18,
                    observed_bytes: 18,
                    truncated: false,
                }),
                stderr: None,
            }],
        };
        let rendered = render_output(&output);
        assert!(rendered.contains("output: generation 7"));
        assert!(rendered.contains("task: my tests"));
        assert!(rendered.contains("stdout: 2 lines, retained 18 bytes, observed 18 bytes:"));
        assert!(rendered.contains("line one"));
        assert!(rendered.contains("line two"));
    }

    #[test]
    fn render_await_includes_failure_evidence_when_present() {
        use crate::control_client::FailureEvidenceSnapshot;
        let mut observation = AwaitSnapshot {
            terminal_reason: "failed".to_string(),
            latest_generation: 7,
            latest_batch: None,
            pending_work: crate::control_client::PendingWorkSnapshot {
                debounce_active: false,
                queued_batches: 0,
            },
            freshness: "current".to_string(),
            snapshot: StatusSnapshot {
                generation: 7,
                state: "failed".to_string(),
                trigger: Some("src/main.rs".to_string()),
                commands: vec!["cargo test".to_string()],
                duration_ms: Some(42),
                failures: vec!["boom".to_string()],
            },
            failure_evidence: Some(FailureEvidenceSnapshot {
                excerpt: "error: boom\ndetail\n".to_string(),
                lines: 2,
                truncated: false,
                total_observed_bytes: 24,
                retained_bytes: 24,
                retrieve: "fzz control output --generation 7 --task 'my tests' --tail 80"
                    .to_string(),
            }),
        };
        let rendered = render_await(&mut observation);
        assert!(rendered.contains("failure evidence:"));
        assert!(rendered.contains("error: boom"));
        assert!(rendered.contains("retrieve: fzz control output --generation 7"));
    }

    #[test]
    fn parse_duration_accepts_units_and_bare_seconds() {
        assert_eq!(parse_duration("500ms").unwrap(), Duration::from_millis(500));
        assert_eq!(parse_duration("2s").unwrap(), Duration::from_secs(2));
        assert_eq!(parse_duration("3m").unwrap(), Duration::from_secs(180));
        assert_eq!(parse_duration("30").unwrap(), Duration::from_secs(30));
    }

    #[test]
    fn parse_duration_rejects_unknown_and_zero_bounds() {
        assert!(parse_duration("1h").is_err());
        assert!(parse_duration("abc").is_err());
        assert!(parse_duration("0s").is_err());
        assert!(parse_duration("").is_err());
    }

    #[test]
    fn render_targets_shows_count_names_and_commands() {
        let targets = vec![
            TargetSnapshot {
                name: "final checks @agent-final".to_string(),
                commands: vec!["cargo test".to_string()],
                estimate: None,
            },
            TargetSnapshot {
                name: "fast tests".to_string(),
                commands: vec!["true".to_string()],
                estimate: None,
            },
        ];
        let rendered = render_targets(&targets);
        assert!(rendered.contains("targets (2):"));
        assert!(rendered.contains("  - final checks @agent-final"));
        assert!(rendered.contains("      cargo test"));
        assert!(rendered.contains("  - fast tests"));
    }

    #[test]
    fn render_targets_shows_deterministic_estimate_line() {
        use crate::duration_history::{EstimateConfidence, EstimateSource};
        let targets = vec![TargetSnapshot {
            name: "final checks @agent-final".to_string(),
            commands: vec!["cargo test".to_string()],
            estimate: Some(RunEstimate {
                typical_ms: 38_000,
                upper_ms: 61_000,
                recommended_timeout_ms: 95_000,
                samples: 12,
                confidence: EstimateConfidence::Medium,
                source: EstimateSource::Measured,
            }),
        }];
        let rendered = render_targets(&targets);
        assert!(rendered.contains(
            "estimate: typical=38.0s upper=1m1 timeout=1m35 confidence=medium n=12 source=measured"
        ));
    }

    #[test]
    fn format_duration_is_deterministic_human_readable() {
        use crate::cli::control::format_duration;
        assert_eq!(format_duration(42), "42ms");
        assert_eq!(format_duration(1_500), "1.5s");
        assert_eq!(format_duration(61_000), "1m1");
        assert_eq!(format_duration(7_260_000), "2h1m");
    }

    #[test]
    fn render_targets_states_empty_explicitly() {
        let rendered = render_targets(&[]);
        assert_eq!(rendered, "targets: (none)\n");
    }

    #[test]
    fn render_run_returns_generation_identity() {
        assert_eq!(render_run(7), "scheduled generation: 7\n");
    }

    #[test]
    fn render_capabilities_lists_protocol_facts() {
        use crate::control_client::{CapabilityFeatures, CapabilityLimits};
        let caps = CapabilitiesSnapshot {
            token: "fz-token".to_string(),
            protocol_version: "1.0".to_string(),
            schema_version: 1,
            watcher_version: "1.6.0".to_string(),
            methods: vec!["status".to_string(), "cancel".to_string()],
            optional_fields: vec!["batch".to_string()],
            output_formats: vec!["toon".to_string(), "json".to_string()],
            limits: CapabilityLimits {
                output_retention_bytes: 1_048_576,
                max_response_bytes: 65_536,
                max_evidence_lines: 40,
                estimate_max_samples: 20,
                estimate_floor_ms: 10_000,
                estimate_cap_ms: 900_000,
            },
            features: CapabilityFeatures {
                atomic_await: true,
                subscription: false,
                correlated_snapshots: false,
                output_retrieval: true,
                pending_work: false,
                duration_estimates: true,
            },
        };
        let rendered = render_capabilities(&caps);
        assert!(rendered.contains("protocol version: 1.0"));
        assert!(rendered.contains("watcher version: 1.6.0"));
        assert!(rendered.contains("instance token: fz-token"));
        assert!(rendered.contains("  - cancel"));
        assert!(rendered.contains("  - json"));
        assert!(rendered.contains("output retention bytes: 1048576"));
        assert!(rendered.contains("atomic await: true"));
        assert!(rendered.contains("subscription: false"));
        assert!(rendered.contains("estimate max samples: 20"));
        assert!(rendered.contains("duration estimates: true"));
    }

    #[test]
    fn render_cancel_reports_cancelled_or_not_running() {
        assert_eq!(
            render_cancel(&CancelSnapshot {
                cancelled: true,
                generation: 7
            }),
            "generation: 7\noutcome: cancelled\n"
        );
        assert_eq!(
            render_cancel(&CancelSnapshot {
                cancelled: false,
                generation: 7
            }),
            "generation: 7\noutcome: not-running\n"
        );
    }

    #[test]
    fn render_emit_scheduled_shows_matched_and_generation() {
        let emit = EmitSnapshot {
            matched: vec!["fast tests".to_string(), "full tests".to_string()],
            run_id: Some(7),
            outcome: "scheduled".to_string(),
        };
        let rendered = render_emit(&emit);
        assert!(rendered.contains("outcome: scheduled"));
        assert!(rendered.contains("matched (2):"));
        assert!(rendered.contains("  - fast tests"));
        assert!(rendered.contains("  - full tests"));
        assert!(rendered.contains("scheduled generation: 7"));
    }

    #[test]
    fn render_emit_unmatched_is_explicit_no_generation() {
        let emit = EmitSnapshot {
            matched: vec![],
            run_id: None,
            outcome: "unmatched".to_string(),
        };
        let rendered = render_emit(&emit);
        assert!(rendered.contains("outcome: unmatched"));
        assert!(rendered.contains("matched: (none)"));
        assert!(!rendered.contains("scheduled generation"));
    }

    #[test]
    fn render_emit_ignored_is_explicit_no_generation() {
        let emit = EmitSnapshot {
            matched: vec![],
            run_id: None,
            outcome: "ignored".to_string(),
        };
        let rendered = render_emit(&emit);
        assert!(rendered.contains("outcome: ignored"));
        assert!(rendered.contains("matched: (none)"));
    }

    #[test]
    fn socket_override_beats_global_and_config() {
        let command = ControlCommand::new(
            ControlAction::Status,
            Some("/tmp/override.sock".to_string()),
            Some("/tmp/global.sock".to_string()),
            Some("config.yaml".to_string()),
        );
        assert_eq!(
            command.resolve_socket().unwrap(),
            PathBuf::from("/tmp/override.sock")
        );
    }

    #[test]
    fn global_socket_beats_config() {
        let command = ControlCommand::new(
            ControlAction::Status,
            None,
            Some("/tmp/global.sock".to_string()),
            Some("config.yaml".to_string()),
        );
        assert_eq!(
            command.resolve_socket().unwrap(),
            PathBuf::from("/tmp/global.sock")
        );
    }

    #[test]
    fn empty_overrides_are_ignored_in_favor_of_config() {
        // Config exists but lacks `on.socket`; resolution must fail with the
        // actionable hint naming the override surface.
        let directory = std::env::temp_dir().join(format!(
            "fzz-control-resolve-override-{}.tmp",
            std::process::id()
        ));
        let _ = std::fs::create_dir_all(&directory);
        let config_path = directory.join(".watch.yaml");
        std::fs::write(&config_path, "tasks:\n  - name: x\n    run: true\n").unwrap();
        let command = ControlCommand::new(
            ControlAction::Status,
            Some("   ".to_string()),
            Some("".to_string()),
            Some(config_path.to_string_lossy().to_string()),
        );
        let err = command.resolve_socket().expect_err("no socket anywhere");
        assert!(err.contains("--socket"));
        assert!(err.contains("on.socket"));
        let _ = std::fs::remove_dir_all(&directory);
    }

    #[test]
    fn missing_explicit_config_reports_the_file() {
        let command = ControlCommand::new(
            ControlAction::Status,
            None,
            None,
            Some("/nonexistent/config.yaml".to_string()),
        );
        let err = command.resolve_socket().expect_err("missing config");
        assert!(err.contains("config file not found"));
        assert!(err.contains("/nonexistent/config.yaml"));
    }

    #[test]
    fn config_socket_used_when_no_override() {
        let directory =
            std::env::temp_dir().join(format!("fzz-control-resolve-{}.tmp", std::process::id()));
        let _ = std::fs::create_dir_all(&directory);
        let config_path = directory.join(".watch.yaml");
        std::fs::write(&config_path, "on:\n  socket: .tmp/control.sock\n").unwrap();
        let command = ControlCommand::new(
            ControlAction::Status,
            None,
            None,
            Some(config_path.to_string_lossy().to_string()),
        );
        assert_eq!(
            command.resolve_socket().unwrap(),
            PathBuf::from(".tmp/control.sock")
        );
        let _ = std::fs::remove_dir_all(&directory);
    }

    #[test]
    fn config_without_socket_falls_through_to_error() {
        let directory = std::env::temp_dir().join(format!(
            "fzz-control-resolve-none-{}.tmp",
            std::process::id()
        ));
        let _ = std::fs::create_dir_all(&directory);
        let config_path = directory.join(".watch.yaml");
        std::fs::write(&config_path, "tasks:\n  - name: x\n    run: true\n").unwrap();
        let command = ControlCommand::new(
            ControlAction::Status,
            None,
            None,
            Some(config_path.to_string_lossy().to_string()),
        );
        let err = command.resolve_socket().expect_err("no on.socket");
        assert!(err.contains("no control socket configured"));
        let _ = std::fs::remove_dir_all(&directory);
    }
}
