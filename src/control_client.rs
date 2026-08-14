//! JSON-RPC 2.0 NDJSON client adapter for the Funzzy control socket.
//!
//! Owns transport and protocol concerns only: connect, NDJSON framing,
//! request IDs, error-object handling, and response validation. CLI
//! presentation never parses protocol shapes; it consumes the typed
//! snapshots produced here. Per TASK-0021 the client is bounded: every
//! read/write carries a timeout and every response is validated against
//! the additive contract in `docs/AGENT-FEEDBACK-CONTRACT.md`.

use crate::duration_history::RunEstimate;
use serde_json::Value;
use std::fmt;
use std::io::{BufRead, BufReader, Write};
use std::os::unix::net::UnixStream;
use std::path::{Path, PathBuf};
use std::time::Duration;

/// Default bound for a single socket read/write operation.
pub const DEFAULT_IO_TIMEOUT_MS: u64 = 3_000;

/// Extra read bound beyond the requested server-side await, so a legitimate
/// wait is never mistaken for a client-side timeout.
pub const AWAIT_READ_MARGIN_MS: u64 = 2_000;

/// A client-visible control failure with a deterministic, actionable message.
#[derive(Debug)]
pub enum ControlClientError {
    /// The socket cannot be reached at the given path.
    Unavailable { path: PathBuf, reason: String },
    /// I/O failure while communicating (other than timeout).
    Io(String),
    /// The server closed the connection without a response (watcher died or
    /// a restart replaced the instance mid-call).
    Disconnected,
    /// No response arrived within the bounded read timeout.
    Timeout,
    /// Response is not valid JSON-RPC 2.0 or drifted from the contract shape.
    Malformed(String),
    /// The server returned a JSON-RPC error object.
    Server {
        code: i64,
        message: String,
        data: Option<Value>,
    },
}

impl fmt::Display for ControlClientError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            ControlClientError::Unavailable { path, reason } => write!(
                f,
                "cannot reach control socket at {}: {}",
                path.display(),
                reason
            ),
            ControlClientError::Io(reason) => write!(f, "control socket I/O error: {}", reason),
            ControlClientError::Disconnected => {
                write!(f, "control socket closed the connection")
            }
            ControlClientError::Timeout => {
                write!(f, "control socket timed out waiting for a response")
            }
            ControlClientError::Malformed(reason) => {
                write!(f, "malformed control socket response: {}", reason)
            }
            ControlClientError::Server {
                code,
                message,
                data,
            } if *code == -32601 => write!(
                f,
                "control socket server error -32601: {} — the running watcher may be an older version without this method; upgrade funzzy to enable it",
                message
            ),
            ControlClientError::Server {
                code,
                message,
                data,
            } => match data {
                Some(Value::String(detail)) if !detail.is_empty() => write!(
                    f,
                    "control socket server error {}: {} ({})",
                    code, message, detail
                ),
                _ => write!(f, "control socket server error {}: {}", code, message),
            },
        }
    }
}

/// Validated `status` result (additive contract §7 legacy shape, preserved).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct StatusSnapshot {
    pub generation: u64,
    pub state: String,
    pub trigger: Option<String>,
    pub commands: Vec<String>,
    pub duration_ms: Option<u64>,
    pub failures: Vec<String>,
    /// Effective concurrency of the latest generation (TASK-0073): None on
    /// legacy servers or when the configured bound applied (additive).
    pub effective_concurrency: Option<u64>,
    /// Override source label (TASK-0073): "control" for an exact control
    /// generation override; None otherwise (additive).
    pub concurrency_source: Option<String>,
}

impl StatusSnapshot {
    fn from_value(value: Value) -> Result<Self, String> {
        let object = value
            .as_object()
            .ok_or_else(|| "status result must be an object".to_string())?;
        let generation = object
            .get("generation")
            .and_then(Value::as_u64)
            .ok_or_else(|| "status result field \"generation\" must be a number".to_string())?;
        let state = object
            .get("state")
            .and_then(Value::as_str)
            .map(str::to_owned)
            .ok_or_else(|| "status result field \"state\" must be a string".to_string())?;
        let trigger = match object.get("trigger") {
            None => None,
            Some(Value::Null) => None,
            Some(Value::String(value)) => Some(value.clone()),
            Some(_) => {
                return Err("status result field \"trigger\" must be a string or null".to_string())
            }
        };
        let commands = read_string_array(object, "commands")?;
        let duration_ms = match object.get("durationMs") {
            None => None,
            Some(Value::Null) => None,
            Some(Value::Number(value)) => value
                .as_u64()
                .map(Some)
                .ok_or_else(|| "status result field \"durationMs\" must be a number".to_string())?,
            Some(_) => {
                return Err(
                    "status result field \"durationMs\" must be a number or null".to_string(),
                )
            }
        };
        let failures = read_string_array(object, "failures")?;
        let effective_concurrency = match object.get("effectiveConcurrency") {
            None | Some(Value::Null) => None,
            Some(Value::Number(value)) => value.as_u64().map(Some).ok_or_else(|| {
                "status result field \"effectiveConcurrency\" must be a number".to_string()
            })?,
            Some(_) => {
                return Err(
                    "status result field \"effectiveConcurrency\" must be a number".to_string(),
                )
            }
        };
        let concurrency_source = match object.get("concurrencySource") {
            None | Some(Value::Null) => None,
            Some(Value::String(value)) => Some(value.clone()),
            Some(_) => {
                return Err("status result field \"concurrencySource\" must be a string".to_string())
            }
        };
        Ok(Self {
            generation,
            state,
            trigger,
            commands,
            duration_ms,
            failures,
            effective_concurrency,
            concurrency_source,
        })
    }
}

/// Validated `targets` result entry.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TargetSnapshot {
    pub name: String,
    pub commands: Vec<String>,
    /// Optional duration estimate (TASK-0055): absent for legacy servers and
    /// targets without history.
    pub estimate: Option<RunEstimate>,
}

impl TargetSnapshot {
    fn from_value(value: Value) -> Result<Self, String> {
        let object = value
            .as_object()
            .ok_or_else(|| "target entry must be an object".to_string())?;
        let name = object
            .get("name")
            .and_then(Value::as_str)
            .map(str::to_owned)
            .ok_or_else(|| "target entry field \"name\" must be a string".to_string())?;
        let commands = read_string_array(object, "commands")?;
        let estimate = object
            .get("estimate")
            .and_then(Value::as_object)
            .map(|estimate| RunEstimate {
                typical_ms: estimate
                    .get("typicalMs")
                    .and_then(Value::as_u64)
                    .unwrap_or_default(),
                upper_ms: estimate
                    .get("upperMs")
                    .and_then(Value::as_u64)
                    .unwrap_or_default(),
                recommended_timeout_ms: estimate
                    .get("recommendedTimeoutMs")
                    .and_then(Value::as_u64)
                    .unwrap_or_default(),
                samples: estimate
                    .get("samples")
                    .and_then(Value::as_u64)
                    .unwrap_or_default() as usize,
                confidence: match estimate.get("confidence").and_then(Value::as_str) {
                    Some("low") => crate::duration_history::EstimateConfidence::Low,
                    Some("medium") => crate::duration_history::EstimateConfidence::Medium,
                    Some("high") => crate::duration_history::EstimateConfidence::High,
                    _ => crate::duration_history::EstimateConfidence::None,
                },
                source: if estimate.get("source").and_then(Value::as_str) == Some("configured") {
                    crate::duration_history::EstimateSource::Configured
                } else {
                    crate::duration_history::EstimateSource::Measured
                },
            });
        Ok(Self {
            name,
            commands,
            estimate,
        })
    }
}

/// What the client asks the server to await (contract §4): the next terminal
/// generation after `N`, or the exact generation `N`.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum AwaitMode {
    After(u64),
    Exact(u64),
}

/// Negotiated capabilities (contract §6): protocol facts a client uses to
/// gate methods and shapes instead of guessing from package versions. The
/// instance token identifies one watcher process so clients can detect
/// restarts instead of assuming continuity.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CapabilitiesSnapshot {
    pub token: String,
    pub protocol_version: String,
    pub schema_version: u64,
    pub watcher_version: String,
    pub methods: Vec<String>,
    pub optional_fields: Vec<String>,
    pub output_formats: Vec<String>,
    pub limits: CapabilityLimits,
    pub features: CapabilityFeatures,
}

/// Declared bounds a client must respect (contract §6); `0` = absent.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct CapabilityLimits {
    pub output_retention_bytes: u64,
    pub max_response_bytes: u64,
    pub max_evidence_lines: u64,
    /// Duration-estimate bounds (TASK-0055): 0 = surface absent. Mirrors the
    /// estimator's retention/floor/cap so clients clamp without guessing.
    pub estimate_max_samples: u64,
    pub estimate_floor_ms: u64,
    pub estimate_cap_ms: u64,
}

/// Negotiated feature flags: each stays false until its implementation lands,
/// so clients keep the legacy fallback for anything not yet supported.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct CapabilityFeatures {
    pub atomic_await: bool,
    pub subscription: bool,
    pub correlated_snapshots: bool,
    pub output_retrieval: bool,
    pub pending_work: bool,
    pub duration_estimates: bool,
    /// Exact-generation sequential override (TASK-0073); absent on legacy
    /// servers, so clients must check before sending `sequential`.
    pub sequential_override: bool,
}

impl CapabilitiesSnapshot {
    fn from_value(value: Value) -> Result<Self, String> {
        let object = value
            .as_object()
            .ok_or_else(|| "capabilities result must be an object".to_string())?;
        let instance = object
            .get("instance")
            .and_then(Value::as_object)
            .ok_or_else(|| "capabilities result must carry \"instance\"".to_string())?;
        let token = instance
            .get("token")
            .and_then(Value::as_str)
            .map(str::to_owned)
            .ok_or_else(|| "capabilities instance must carry a \"token\" string".to_string())?;
        let protocol_version = object
            .get("protocolVersion")
            .and_then(Value::as_str)
            .map(str::to_owned)
            .unwrap_or_default();
        let schema_version = object
            .get("schemaVersion")
            .and_then(Value::as_u64)
            .unwrap_or_default();
        let watcher_version = object
            .get("watcherVersion")
            .and_then(Value::as_str)
            .map(str::to_owned)
            .unwrap_or_default();
        let methods = read_string_array(object, "methods")?;
        let optional_fields = read_string_array(object, "optionalFields")?;
        let output_formats = read_string_array(object, "outputFormats")?;
        let limits = CapabilityLimits {
            output_retention_bytes: object
                .get("limits")
                .and_then(Value::as_object)
                .and_then(|limits| limits.get("outputRetentionBytes"))
                .and_then(Value::as_u64)
                .unwrap_or_default(),
            max_response_bytes: object
                .get("limits")
                .and_then(Value::as_object)
                .and_then(|limits| limits.get("maxResponseBytes"))
                .and_then(Value::as_u64)
                .unwrap_or_default(),
            max_evidence_lines: object
                .get("limits")
                .and_then(Value::as_object)
                .and_then(|limits| limits.get("maxEvidenceLines"))
                .and_then(Value::as_u64)
                .unwrap_or_default(),
            estimate_max_samples: object
                .get("limits")
                .and_then(Value::as_object)
                .and_then(|limits| limits.get("durationEstimateLimits"))
                .and_then(Value::as_object)
                .and_then(|limits| limits.get("maxSamples"))
                .and_then(Value::as_u64)
                .unwrap_or_default(),
            estimate_floor_ms: object
                .get("limits")
                .and_then(Value::as_object)
                .and_then(|limits| limits.get("durationEstimateLimits"))
                .and_then(Value::as_object)
                .and_then(|limits| limits.get("floorMs"))
                .and_then(Value::as_u64)
                .unwrap_or_default(),
            estimate_cap_ms: object
                .get("limits")
                .and_then(Value::as_object)
                .and_then(|limits| limits.get("durationEstimateLimits"))
                .and_then(Value::as_object)
                .and_then(|limits| limits.get("capMs"))
                .and_then(Value::as_u64)
                .unwrap_or_default(),
        };
        let features = CapabilityFeatures {
            atomic_await: read_feature(object, "atomicAwait"),
            subscription: read_feature(object, "subscription"),
            correlated_snapshots: read_feature(object, "correlatedSnapshots"),
            output_retrieval: read_feature(object, "outputRetrieval"),
            pending_work: read_feature(object, "pendingWork"),
            duration_estimates: read_feature(object, "durationEstimates"),
            sequential_override: read_feature(object, "sequentialOverride"),
        };
        Ok(Self {
            token,
            protocol_version,
            schema_version,
            watcher_version,
            methods,
            optional_fields,
            output_formats,
            limits,
            features,
        })
    }

    /// Whether the negotiated profile advertises a method, so clients gate
    /// await/emit/cancel/output on facts rather than trial side effects.
    pub fn has_method(&self, method: &str) -> bool {
        self.methods.iter().any(|m| m == method)
    }
}

fn read_feature(object: &serde_json::Map<String, Value>, key: &str) -> bool {
    object
        .get("features")
        .and_then(Value::as_object)
        .and_then(|features| features.get(key))
        .and_then(Value::as_bool)
        .unwrap_or(false)
}

/// Pending debounce work (contract §3 freshness rule).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PendingWorkSnapshot {
    pub debounce_active: bool,
    pub queued_batches: u32,
}

/// One consistent await observation (contract §4): a snapshot plus the
/// terminal reason, latest observed batch/generation, pending debounce state,
/// and freshness classification.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AwaitSnapshot {
    pub terminal_reason: String,
    pub latest_generation: u64,
    pub latest_batch: Option<u64>,
    pub pending_work: PendingWorkSnapshot,
    pub freshness: String,
    pub snapshot: StatusSnapshot,
    /// Concise failure evidence (contract §6), present when the awaited
    /// generation failed and retained output exists (additive; absent on
    /// legacy servers).
    pub failure_evidence: Option<FailureEvidenceSnapshot>,
}

/// Concise deterministic failure excerpt plus bounds and retrieval hint.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct FailureEvidenceSnapshot {
    pub excerpt: String,
    pub lines: u64,
    pub truncated: bool,
    pub total_observed_bytes: u64,
    pub retained_bytes: u64,
    pub retrieve: String,
}

impl FailureEvidenceSnapshot {
    fn from_value(value: Value) -> Result<Self, String> {
        let object = value
            .as_object()
            .ok_or_else(|| "failureEvidence must be an object".to_string())?;
        let excerpt = object
            .get("excerpt")
            .and_then(Value::as_str)
            .map(str::to_owned)
            .ok_or_else(|| "failureEvidence must carry \"excerpt\"".to_string())?;
        let lines = object
            .get("lines")
            .and_then(Value::as_u64)
            .ok_or_else(|| "failureEvidence must carry a numeric \"lines\"".to_string())?;
        let truncated = object
            .get("truncated")
            .and_then(Value::as_bool)
            .ok_or_else(|| "failureEvidence must carry \"truncated\"".to_string())?;
        let total_observed_bytes = object
            .get("totalObservedBytes")
            .and_then(Value::as_u64)
            .ok_or_else(|| "failureEvidence must carry \"totalObservedBytes\"".to_string())?;
        let retained_bytes = object
            .get("retainedBytes")
            .and_then(Value::as_u64)
            .ok_or_else(|| "failureEvidence must carry \"retainedBytes\"".to_string())?;
        let retrieve = object
            .get("retrieve")
            .and_then(Value::as_str)
            .map(str::to_owned)
            .ok_or_else(|| "failureEvidence must carry \"retrieve\"".to_string())?;
        Ok(Self {
            excerpt,
            lines,
            truncated,
            total_observed_bytes,
            retained_bytes,
            retrieve,
        })
    }
}

impl AwaitSnapshot {
    fn from_value(value: Value) -> Result<Self, String> {
        let object = value
            .as_object()
            .ok_or_else(|| "await result must be an object".to_string())?;
        let terminal_reason = object
            .get("terminalReason")
            .and_then(Value::as_str)
            .map(str::to_owned)
            .filter(|reason| {
                matches!(
                    reason.as_str(),
                    "passed"
                        | "failed"
                        | "cancelled"
                        | "superseded"
                        | "timeout"
                        | "disconnected"
                        | "restarted"
                )
            })
            .ok_or_else(|| "await result field \"terminalReason\" is invalid".to_string())?;
        let latest_generation = object
            .get("latestGeneration")
            .and_then(Value::as_u64)
            .ok_or_else(|| {
                "await result field \"latestGeneration\" must be a number".to_string()
            })?;
        let latest_batch = match object.get("latestBatch") {
            None | Some(Value::Null) => None,
            Some(Value::Number(number)) => number.as_u64(),
            Some(_) => {
                return Err(
                    "await result field \"latestBatch\" must be a number or null".to_string(),
                )
            }
        };
        let pending = object
            .get("pendingWork")
            .and_then(Value::as_object)
            .ok_or_else(|| "await result must carry a \"pendingWork\" object".to_string())?;
        let pending_work = PendingWorkSnapshot {
            debounce_active: pending
                .get("debounceActive")
                .and_then(Value::as_bool)
                .ok_or_else(|| "pendingWork.debounceActive must be a boolean".to_string())?,
            queued_batches: pending
                .get("queuedBatches")
                .and_then(Value::as_u64)
                .map(|count| count as u32)
                .ok_or_else(|| "pendingWork.queuedBatches must be a number".to_string())?,
        };
        let freshness = object
            .get("freshness")
            .and_then(Value::as_str)
            .map(str::to_owned)
            .filter(|freshness| matches!(freshness.as_str(), "current" | "stale" | "unknown"))
            .ok_or_else(|| "await result field \"freshness\" is invalid".to_string())?;
        let snapshot = object
            .get("snapshot")
            .cloned()
            .ok_or_else(|| "await result must carry a \"snapshot\" object".to_string())?;
        let snapshot = StatusSnapshot::from_value(snapshot)?;
        let failure_evidence = match object.get("failureEvidence") {
            None | Some(Value::Null) => None,
            Some(value) => Some(FailureEvidenceSnapshot::from_value(value.clone())?),
        };
        Ok(Self {
            terminal_reason,
            latest_generation,
            latest_batch,
            pending_work,
            freshness,
            snapshot,
            failure_evidence,
        })
    }
}

/// One retrieved stream plus bounds metadata (contract §6).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct StreamSnapshot {
    pub content: String,
    pub lines: u64,
    pub retained_bytes: u64,
    pub observed_bytes: u64,
    pub truncated: bool,
}

impl StreamSnapshot {
    fn from_value(value: Value) -> Result<Self, String> {
        let object = value
            .as_object()
            .ok_or_else(|| "output stream must be an object".to_string())?;
        let content = object
            .get("content")
            .and_then(Value::as_str)
            .map(str::to_owned)
            .ok_or_else(|| "output stream must carry \"content\"".to_string())?;
        let lines = object
            .get("lines")
            .and_then(Value::as_u64)
            .ok_or_else(|| "output stream must carry a numeric \"lines\"".to_string())?;
        let retained_bytes = object
            .get("retainedBytes")
            .and_then(Value::as_u64)
            .ok_or_else(|| "output stream must carry \"retainedBytes\"".to_string())?;
        let observed_bytes = object
            .get("observedBytes")
            .and_then(Value::as_u64)
            .ok_or_else(|| "output stream must carry \"observedBytes\"".to_string())?;
        let truncated = object
            .get("truncated")
            .and_then(Value::as_bool)
            .ok_or_else(|| "output stream must carry \"truncated\"".to_string())?;
        Ok(Self {
            content,
            lines,
            retained_bytes,
            observed_bytes,
            truncated,
        })
    }
}

/// One retrieved task's streams.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RetrievedTaskSnapshot {
    pub id: String,
    pub stdout: Option<StreamSnapshot>,
    pub stderr: Option<StreamSnapshot>,
}

/// Retrieval domain result (contract §6): consumed by the renderers.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct OutputSnapshot {
    pub generation: u64,
    pub tasks: Vec<RetrievedTaskSnapshot>,
}

impl OutputSnapshot {
    fn from_value(value: Value) -> Result<Self, String> {
        let object = value
            .as_object()
            .ok_or_else(|| "output result must be an object".to_string())?;
        let generation = object
            .get("generation")
            .and_then(Value::as_u64)
            .ok_or_else(|| "output result must carry a numeric \"generation\"".to_string())?;
        let tasks = object
            .get("tasks")
            .and_then(Value::as_array)
            .ok_or_else(|| "output result must carry a \"tasks\" array".to_string())?
            .iter()
            .map(|task| {
                let task = task
                    .as_object()
                    .ok_or_else(|| "task must be an object".to_string())?;
                let id = task
                    .get("id")
                    .and_then(Value::as_str)
                    .map(str::to_owned)
                    .ok_or_else(|| "task must carry \"id\"".to_string())?;
                let stdout = match task.get("stdout") {
                    None | Some(Value::Null) => None,
                    Some(value) => Some(StreamSnapshot::from_value(value.clone())?),
                };
                let stderr = match task.get("stderr") {
                    None | Some(Value::Null) => None,
                    Some(value) => Some(StreamSnapshot::from_value(value.clone())?),
                };
                Ok(RetrievedTaskSnapshot { id, stdout, stderr })
            })
            .collect::<Result<Vec<_>, String>>()?;
        Ok(Self { generation, tasks })
    }
}

/// Validated `emit` result (contract §5): matched task names plus the
/// scheduled generation, or an explicit unmatched/ignored outcome.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct EmitSnapshot {
    pub matched: Vec<String>,
    pub run_id: Option<u64>,
    pub outcome: String,
}

impl EmitSnapshot {
    fn from_value(value: Value) -> Result<Self, String> {
        let object = value
            .as_object()
            .ok_or_else(|| "emit result must be an object".to_string())?;
        let outcome = object
            .get("outcome")
            .and_then(Value::as_str)
            .map(str::to_owned)
            .ok_or_else(|| "emit result field \"outcome\" must be a string".to_string())?;
        if !matches!(outcome.as_str(), "scheduled" | "unmatched" | "ignored") {
            return Err(format!(
                "emit result field \"outcome\" must be scheduled, unmatched, or ignored, got {}",
                outcome
            ));
        }
        let matched = read_string_array(object, "matched")?;
        let run_id = match object.get("runId") {
            None => None,
            Some(Value::Null) => None,
            Some(Value::Number(value)) => value
                .as_u64()
                .map(Some)
                .ok_or_else(|| "emit result field \"runId\" must be a number".to_string())?,
            Some(_) => {
                return Err("emit result field \"runId\" must be a number or null".to_string())
            }
        };
        Ok(Self {
            matched,
            run_id,
            outcome,
        })
    }
}

/// Validated `cancel` result (contract §10): `{ cancelled: bool, generation: u64 }`.
/// `cancelled: false` is a no-op (already terminal or unknown); escalation
/// arrives as RPC error -32021 instead of this shape.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CancelSnapshot {
    pub cancelled: bool,
    pub generation: u64,
}

impl CancelSnapshot {
    fn from_value(value: Value) -> Result<Self, String> {
        let object = value
            .as_object()
            .ok_or_else(|| "cancel result must be an object".to_string())?;
        let cancelled = object
            .get("cancelled")
            .and_then(Value::as_bool)
            .ok_or_else(|| "cancel result field \"cancelled\" must be a boolean".to_string())?;
        let generation = object
            .get("generation")
            .and_then(Value::as_u64)
            .ok_or_else(|| "cancel result field \"generation\" must be a number".to_string())?;
        Ok(Self {
            cancelled,
            generation,
        })
    }
}

fn read_string_array(
    object: &serde_json::Map<String, Value>,
    field: &str,
) -> Result<Vec<String>, String> {
    match object.get(field) {
        None => Ok(Vec::new()),
        Some(Value::Array(values)) => values
            .iter()
            .map(|value| {
                value
                    .as_str()
                    .map(str::to_owned)
                    .ok_or_else(|| format!("{field} must contain only strings"))
            })
            .collect(),
        Some(_) => Err(format!("{field} must be an array of strings")),
    }
}

/// Bounded JSON-RPC 2.0 client over one Unix socket connection.
#[derive(Debug)]
pub struct ControlClient {
    reader: BufReader<UnixStream>,
    writer: UnixStream,
    next_id: u64,
}

impl ControlClient {
    /// Connects with the default bounded I/O timeout.
    pub fn connect(path: &Path) -> Result<Self, ControlClientError> {
        Self::connect_with_timeout(path, Duration::from_millis(DEFAULT_IO_TIMEOUT_MS))
    }

    /// Connects with an explicit per-operation timeout (used by tests and
    /// future `--timeout` flags).
    pub fn connect_with_timeout(
        path: &Path,
        timeout: Duration,
    ) -> Result<Self, ControlClientError> {
        let stream = UnixStream::connect(path).map_err(|err| ControlClientError::Unavailable {
            path: path.to_path_buf(),
            reason: err.to_string(),
        })?;
        stream
            .set_read_timeout(Some(timeout))
            .map_err(|err| ControlClientError::Io(err.to_string()))?;
        stream
            .set_write_timeout(Some(timeout))
            .map_err(|err| ControlClientError::Io(err.to_string()))?;
        let reader = BufReader::new(
            stream
                .try_clone()
                .map_err(|err| ControlClientError::Io(err.to_string()))?,
        );
        Ok(Self {
            reader,
            writer: stream,
            next_id: 0,
        })
    }

    /// Requests `status`; validates the response shape into a snapshot.
    pub fn status(&mut self) -> Result<StatusSnapshot, ControlClientError> {
        let result = self.call("status", serde_json::json!({}))?;
        StatusSnapshot::from_value(result).map_err(ControlClientError::Malformed)
    }

    /// Requests `targets`; validates the response into a target list.
    pub fn targets(&mut self) -> Result<Vec<TargetSnapshot>, ControlClientError> {
        let result = self.call("targets", serde_json::json!({}))?;
        let values = result
            .as_array()
            .ok_or_else(|| {
                ControlClientError::Malformed("targets result must be an array".to_string())
            })?
            .iter()
            .cloned()
            .map(TargetSnapshot::from_value)
            .collect::<Result<Vec<_>, _>>()
            .map_err(ControlClientError::Malformed)?;
        Ok(values)
    }

    /// Requests `run` for an exact target; returns the scheduled generation.
    /// `sequential` (TASK-0073) is an additive typed parameter requesting
    /// effective concurrency one for this exact generation.
    pub fn run(&mut self, target: &str, sequential: bool) -> Result<u64, ControlClientError> {
        let params = serde_json::json!({ "target": target, "sequential": sequential });
        let result = self.call("run", params)?;
        result.get("runId").and_then(Value::as_u64).ok_or_else(|| {
            ControlClientError::Malformed("run result must carry a numeric \"runId\"".to_string())
        })
    }

    /// Requests `emit` for one path; returns matched tasks and run identity
    /// or an explicit unmatched/ignored outcome.
    pub fn emit(&mut self, path: &str) -> Result<EmitSnapshot, ControlClientError> {
        let result = self.call("emit", serde_json::json!({ "path": path }))?;
        EmitSnapshot::from_value(result).map_err(ControlClientError::Malformed)
    }

    /// Negotiates `capabilities` (contract §6): returns the instance token so
    /// callers can detect watcher restarts.
    pub fn capabilities(&mut self) -> Result<CapabilitiesSnapshot, ControlClientError> {
        let result = self.call("capabilities", serde_json::json!({}))?;
        CapabilitiesSnapshot::from_value(result).map_err(ControlClientError::Malformed)
    }

    /// Atomic await (contract §4): blocks server-side until the mode's
    /// condition or the bound, then returns one consistent snapshot with
    /// terminal reason and freshness. The socket read bound is extended to
    /// cover the wait, so a legitimate wait is never a client timeout.
    pub fn await_generation(
        &mut self,
        mode: AwaitMode,
        timeout_ms: u64,
    ) -> Result<AwaitSnapshot, ControlClientError> {
        let params = match mode {
            AwaitMode::After(generation) => {
                serde_json::json!({ "after": generation, "timeoutMs": timeout_ms })
            }
            AwaitMode::Exact(generation) => {
                serde_json::json!({ "generation": generation, "timeoutMs": timeout_ms })
            }
        };
        let read_bound = Duration::from_millis(timeout_ms + AWAIT_READ_MARGIN_MS);
        self.reader
            .get_mut()
            .set_read_timeout(Some(read_bound))
            .map_err(|err| ControlClientError::Io(err.to_string()))?;
        let result = self.call("await", params);
        let _ = self
            .reader
            .get_mut()
            .set_read_timeout(Some(Duration::from_millis(DEFAULT_IO_TIMEOUT_MS)));
        let result = result?;
        AwaitSnapshot::from_value(result).map_err(ControlClientError::Malformed)
    }

    /// Compare-and-cancel an exact generation (contract §10): returns whether
    /// the generation matched, or a no-op when it was already terminal or
    /// unknown. `instance_token`, when provided, makes a stale request against
    /// a different watcher process a safe no-op. Escalation (force cleanup)
    /// surfaces as a server error with code -32021.
    pub fn cancel(
        &mut self,
        generation: u64,
        instance_token: Option<&str>,
    ) -> Result<CancelSnapshot, ControlClientError> {
        let mut params = serde_json::json!({ "generation": generation });
        if let Some(token) = instance_token {
            params["instanceToken"] = serde_json::json!(token);
        }
        // Graceful shutdown can run up to the cancel grace period (default
        // 5s), so a normal 3s read bound would misclassify a legitimate
        // escalated cleanup as a client timeout. Bounded generously.
        let read_bound = Duration::from_secs(30);
        self.reader
            .get_mut()
            .set_read_timeout(Some(read_bound))
            .map_err(|err| ControlClientError::Io(err.to_string()))?;
        let result = self.call("cancel", params);
        let _ = self
            .reader
            .get_mut()
            .set_read_timeout(Some(Duration::from_millis(DEFAULT_IO_TIMEOUT_MS)));
        let result = result?;
        CancelSnapshot::from_value(result).map_err(ControlClientError::Malformed)
    }

    /// Retrieves bounded retained output for one generation (contract §6).
    pub fn output(
        &mut self,
        generation: u64,
        task: Option<&str>,
        stream: Option<&str>,
        tail: Option<u64>,
        full: bool,
    ) -> Result<OutputSnapshot, ControlClientError> {
        let mut params = serde_json::json!({ "generation": generation });
        if let Some(task) = task {
            params["task"] = serde_json::json!(task);
        }
        if let Some(stream) = stream {
            params["stream"] = serde_json::json!(stream);
        }
        if let Some(tail) = tail {
            params["tail"] = serde_json::json!(tail);
        }
        if full {
            params["full"] = serde_json::json!(true);
        }
        let result = self.call("output", params)?;
        OutputSnapshot::from_value(result).map_err(ControlClientError::Malformed)
    }

    /// Framing, request ID, error-object handling, and response validation.
    /// Notifications are never produced; every call has an id.
    fn call(&mut self, method: &str, params: Value) -> Result<Value, ControlClientError> {
        self.next_id += 1;
        let id = self.next_id;
        let request =
            serde_json::json!({"jsonrpc": "2.0", "id": id, "method": method, "params": params});
        let mut line = serde_json::to_string(&request)
            .map_err(|err| ControlClientError::Io(err.to_string()))?;
        line.push('\n');
        self.writer.write_all(line.as_bytes()).map_err(io_timeout)?;
        self.writer.flush().map_err(io_timeout)?;

        let mut response = String::new();
        match self.reader.read_line(&mut response) {
            Err(err) => return Err(io_timeout(err)),
            Ok(0) if response.trim().is_empty() => return Err(ControlClientError::Disconnected),
            Ok(_) => {}
        }

        let value: Value = serde_json::from_str(&response)
            .map_err(|err| ControlClientError::Malformed(format!("not valid JSON: {}", err)))?;
        let object = value.as_object().ok_or_else(|| {
            ControlClientError::Malformed("response must be a JSON object".to_string())
        })?;
        if object.get("jsonrpc") != Some(&serde_json::json!("2.0")) {
            return Err(ControlClientError::Malformed(
                "response is not jsonrpc 2.0".to_string(),
            ));
        }
        if object.get("id") != Some(&serde_json::json!(id)) {
            return Err(ControlClientError::Malformed(
                "response id does not match the request id".to_string(),
            ));
        }
        if let Some(error) = object.get("error") {
            let code = error.get("code").and_then(Value::as_i64).ok_or_else(|| {
                ControlClientError::Malformed(
                    "error object must carry a numeric \"code\"".to_string(),
                )
            })?;
            let message = error
                .get("message")
                .and_then(Value::as_str)
                .map(str::to_owned)
                .unwrap_or_default();
            return Err(ControlClientError::Server {
                code,
                message,
                data: error.get("data").cloned(),
            });
        }
        object
            .get("result")
            .cloned()
            .ok_or_else(|| ControlClientError::Malformed("response lacks result".to_string()))
    }
}

/// Maps a socket I/O error: timeout-like conditions become `Timeout`.
fn io_timeout(err: std::io::Error) -> ControlClientError {
    match err.kind() {
        std::io::ErrorKind::WouldBlock | std::io::ErrorKind::TimedOut => {
            ControlClientError::Timeout
        }
        _ => ControlClientError::Io(err.to_string()),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::BufRead;
    use std::os::unix::net::UnixListener;
    use std::sync::atomic::{AtomicU32, Ordering};

    static SOCKET_COUNTER: AtomicU32 = AtomicU32::new(0);

    /// Binds a one-shot listener that reads one request line then writes a
    /// canned response, returning the socket path and the serving thread.
    fn serving_socket(response: String) -> (PathBuf, std::thread::JoinHandle<()>) {
        let counter = SOCKET_COUNTER.fetch_add(1, Ordering::SeqCst);
        let path = std::env::temp_dir().join(format!(
            "fzz-control-client-{}-{counter}.sock",
            std::process::id()
        ));
        let _ = std::fs::remove_file(&path);
        let listener = UnixListener::bind(&path).expect("bind test socket");
        let handle = std::thread::spawn(move || {
            let (stream, _) = listener.accept().expect("accept client");
            let mut reader = BufReader::new(stream.try_clone().expect("clone stream"));
            let mut request = String::new();
            let _ = reader.read_line(&mut request);
            let mut stream = stream;
            use std::io::Write;
            let _ = writeln!(stream, "{}", response);
        });
        (path, handle)
    }

    /// Binds a listener that accepts then never responds (for timeout tests).
    fn silent_socket() -> (PathBuf, std::thread::JoinHandle<()>) {
        let counter = SOCKET_COUNTER.fetch_add(1, Ordering::SeqCst);
        let path = std::env::temp_dir().join(format!(
            "fzz-control-silent-{}-{counter}.sock",
            std::process::id()
        ));
        let _ = std::fs::remove_file(&path);
        let listener = UnixListener::bind(&path).expect("bind test socket");
        let handle = std::thread::spawn(move || {
            let (_stream, _) = listener.accept().expect("accept client");
            std::thread::sleep(Duration::from_secs(5));
        });
        (path, handle)
    }

    fn ok_response(id: u64, result: Value) -> String {
        serde_json::json!({"jsonrpc": "2.0", "id": id, "result": result}).to_string()
    }

    #[test]
    fn status_roundtrip_validates_snapshot() {
        let result = serde_json::json!({
            "generation": 4,
            "state": "passed",
            "trigger": "src/main.rs",
            "commands": ["cargo test"],
            "durationMs": 42,
            "failures": []
        });
        let (path, handle) = serving_socket(ok_response(1, result));
        let mut client = ControlClient::connect(&path).expect("connect");
        let status = client.status().expect("status");
        handle.join().expect("server thread");
        assert_eq!(
            status,
            StatusSnapshot {
                generation: 4,
                state: "passed".to_string(),
                trigger: Some("src/main.rs".to_string()),
                commands: vec!["cargo test".to_string()],
                duration_ms: Some(42),
                failures: vec![],
                effective_concurrency: None,
                concurrency_source: None,
            }
        );
    }

    #[test]
    fn status_accepts_absent_optional_fields() {
        let result = serde_json::json!({"generation": 0, "state": "idle"});
        let (path, handle) = serving_socket(ok_response(1, result));
        let mut client = ControlClient::connect(&path).expect("connect");
        let status = client.status().expect("status");
        handle.join().expect("server thread");
        assert_eq!(status.generation, 0);
        assert_eq!(status.state, "idle");
        assert_eq!(status.trigger, None);
        assert_eq!(status.duration_ms, None);
        assert!(status.commands.is_empty());
        assert!(status.failures.is_empty());
    }

    #[test]
    fn status_missing_required_field_is_malformed() {
        let result = serde_json::json!({"state": "idle"});
        let (path, handle) = serving_socket(ok_response(1, result));
        let mut client = ControlClient::connect(&path).expect("connect");
        let err = client.status().expect_err("missing generation must fail");
        handle.join().expect("server thread");
        assert!(matches!(err, ControlClientError::Malformed(_)));
    }

    #[test]
    fn targets_roundtrip_validates_list() {
        let result = serde_json::json!([
            {"name": "final checks @agent-final", "commands": ["cargo test"]},
            {"name": "fast tests", "commands": ["true"]}
        ]);
        let (path, handle) = serving_socket(ok_response(1, result));
        let mut client = ControlClient::connect(&path).expect("connect");
        let targets = client.targets().expect("targets");
        handle.join().expect("server thread");
        assert_eq!(targets.len(), 2);
        assert_eq!(targets[0].name, "final checks @agent-final");
        assert_eq!(targets[0].commands, vec!["cargo test".to_string()]);
    }

    #[test]
    fn run_returns_scheduled_generation() {
        let result = serde_json::json!({"runId": 7});
        let (path, handle) = serving_socket(ok_response(1, result));
        let mut client = ControlClient::connect(&path).expect("connect");
        let generation = client.run("@agent-final", false).expect("run");
        handle.join().expect("server thread");
        assert_eq!(generation, 7);
    }

    #[test]
    fn run_without_run_id_is_malformed() {
        let result = serde_json::json!({});
        let (path, handle) = serving_socket(ok_response(1, result));
        let mut client = ControlClient::connect(&path).expect("connect");
        let err = client.run("x", false).expect_err("missing runId must fail");
        handle.join().expect("server thread");
        assert!(matches!(err, ControlClientError::Malformed(_)));
    }

    #[test]
    fn emit_scheduled_returns_matched_and_run_id() {
        let result = serde_json::json!({
            "matched": ["fast tests", "full tests"],
            "runId": 7,
            "outcome": "scheduled"
        });
        let (path, handle) = serving_socket(ok_response(1, result));
        let mut client = ControlClient::connect(&path).expect("connect");
        let emit = client.emit("src/main.rs").expect("emit");
        handle.join().expect("server thread");
        assert_eq!(emit.outcome, "scheduled");
        assert_eq!(
            emit.matched,
            vec!["fast tests".to_string(), "full tests".to_string()]
        );
        assert_eq!(emit.run_id, Some(7));
    }

    #[test]
    fn emit_unmatched_is_explicit_no_generation() {
        let result = serde_json::json!({"matched": [], "runId": null, "outcome": "unmatched"});
        let (path, handle) = serving_socket(ok_response(1, result));
        let mut client = ControlClient::connect(&path).expect("connect");
        let emit = client.emit("docs/x.md").expect("emit");
        handle.join().expect("server thread");
        assert_eq!(emit.outcome, "unmatched");
        assert!(emit.matched.is_empty());
        assert_eq!(emit.run_id, None);
    }

    #[test]
    fn emit_ignored_is_explicit_no_generation() {
        let result = serde_json::json!({"matched": [], "runId": null, "outcome": "ignored"});
        let (path, handle) = serving_socket(ok_response(1, result));
        let mut client = ControlClient::connect(&path).expect("connect");
        let emit = client.emit("src/generated/out.rs").expect("emit");
        handle.join().expect("server thread");
        assert_eq!(emit.outcome, "ignored");
        assert_eq!(emit.run_id, None);
    }

    #[test]
    fn emit_unknown_outcome_is_malformed() {
        let result = serde_json::json!({"matched": [], "runId": null, "outcome": "maybe"});
        let (path, handle) = serving_socket(ok_response(1, result));
        let mut client = ControlClient::connect(&path).expect("connect");
        let err = client.emit("x").expect_err("unknown outcome must fail");
        handle.join().expect("server thread");
        assert!(matches!(err, ControlClientError::Malformed(_)));
    }

    #[test]
    fn emit_missing_outcome_is_malformed() {
        let result = serde_json::json!({"matched": [], "runId": 3});
        let (path, handle) = serving_socket(ok_response(1, result));
        let mut client = ControlClient::connect(&path).expect("connect");
        let err = client.emit("x").expect_err("missing outcome must fail");
        handle.join().expect("server thread");
        assert!(matches!(err, ControlClientError::Malformed(_)));
    }

    #[test]
    fn cancel_roundtrip_parses_graceful_ack() {
        let result = serde_json::json!({ "cancelled": true, "generation": 7 });
        let (path, handle) = serving_socket(ok_response(1, result));
        let mut client = ControlClient::connect(&path).expect("connect");
        let snapshot = client.cancel(7, Some("fz-7f3a")).expect("cancel");
        handle.join().unwrap();
        let _ = std::fs::remove_file(&path);
        assert_eq!(
            snapshot,
            CancelSnapshot {
                cancelled: true,
                generation: 7
            }
        );
    }

    #[test]
    fn cancel_noop_roundtrip_parses_cancelled_false() {
        let result = serde_json::json!({ "cancelled": false, "generation": 7 });
        let (path, handle) = serving_socket(ok_response(1, result));
        let mut client = ControlClient::connect(&path).expect("connect");
        let snapshot = client.cancel(7, None).expect("cancel");
        handle.join().unwrap();
        let _ = std::fs::remove_file(&path);
        assert_eq!(
            snapshot,
            CancelSnapshot {
                cancelled: false,
                generation: 7
            }
        );
    }

    #[test]
    fn cancel_malformed_shape_fails_closed() {
        let result = serde_json::json!({ "generation": 7 });
        let (path, handle) = serving_socket(ok_response(1, result));
        let mut client = ControlClient::connect(&path).expect("connect");
        let err = client
            .cancel(7, None)
            .expect_err("missing cancelled must fail");
        handle.join().unwrap();
        let _ = std::fs::remove_file(&path);
        assert!(matches!(err, ControlClientError::Malformed(_)));
    }

    #[test]
    fn server_error_object_surfaces_code_and_message() {
        let response = serde_json::json!({
            "jsonrpc": "2.0",
            "id": 1,
            "error": {"code": -32000, "message": "No target found for 'nope'"}
        })
        .to_string();
        let (path, handle) = serving_socket(response);
        let mut client = ControlClient::connect(&path).expect("connect");
        let err = client
            .run("nope", false)
            .expect_err("server error must surface");
        handle.join().expect("server thread");
        match &err {
            ControlClientError::Server { code, message, .. } => {
                assert_eq!(*code, -32000);
                assert_eq!(message, "No target found for 'nope'");
            }
            other => panic!("expected Server error, got {:?}", other),
        }
        assert!(
            err.to_string().contains("No target found for 'nope'"),
            "actionable detail must surface: {}",
            err
        );
    }

    #[test]
    fn malformed_json_response_fails_closed() {
        let (path, handle) = serving_socket("not json at all".to_string());
        let mut client = ControlClient::connect(&path).expect("connect");
        let err = client.status().expect_err("garbage must fail");
        handle.join().expect("server thread");
        assert!(matches!(err, ControlClientError::Malformed(_)));
    }

    #[test]
    fn id_mismatch_fails_closed() {
        let result = serde_json::json!({"generation": 1, "state": "idle"});
        // Respond with id 99 while the request uses id 1.
        let (path, handle) = serving_socket(ok_response(99, result));
        let mut client = ControlClient::connect(&path).expect("connect");
        let err = client.status().expect_err("id mismatch must fail");
        handle.join().expect("server thread");
        assert!(matches!(err, ControlClientError::Malformed(_)));
    }

    #[test]
    fn non_2_0_response_fails_closed() {
        let response =
            r#"{"jsonrpc":"1.0","id":1,"result":{"generation":1,"state":"idle"}}"#.to_string();
        let (path, handle) = serving_socket(response);
        let mut client = ControlClient::connect(&path).expect("connect");
        let err = client.status().expect_err("wrong jsonrpc must fail");
        handle.join().expect("server thread");
        assert!(matches!(err, ControlClientError::Malformed(_)));
    }

    #[test]
    fn unavailable_socket_reports_selected_path() {
        let path =
            std::env::temp_dir().join(format!("fzz-control-missing-{}.sock", std::process::id()));
        let _ = std::fs::remove_file(&path);
        let err = ControlClient::connect(&path).expect_err("missing socket must fail");
        match &err {
            ControlClientError::Unavailable { path: reported, .. } => {
                assert_eq!(reported, &path);
            }
            other => panic!("expected Unavailable, got {:?}", other),
        }
        assert!(err.to_string().contains("cannot reach control socket"));
        assert!(err
            .to_string()
            .contains(&path.to_string_lossy().to_string()));
    }

    #[test]
    fn silent_server_times_out_within_bound() {
        let (path, handle) = silent_socket();
        let mut client = ControlClient::connect_with_timeout(&path, Duration::from_millis(100))
            .expect("connect");
        let start = std::time::Instant::now();
        let err = client.status().expect_err("silent server must time out");
        let elapsed = start.elapsed();
        handle.join().expect("server thread");
        assert!(matches!(err, ControlClientError::Timeout));
        assert!(
            elapsed < Duration::from_secs(3),
            "timeout took too long: {:?}",
            elapsed
        );
    }

    fn await_result() -> String {
        serde_json::json!({
            "jsonrpc": "2.0",
            "id": 1,
            "result": {
                "snapshot": {
                    "generation": 7,
                    "state": "passed",
                    "trigger": "src/main.rs",
                    "commands": ["cargo test"],
                    "durationMs": 42,
                    "failures": []
                },
                "terminalReason": "passed",
                "latestGeneration": 7,
                "latestBatch": 3,
                "pendingWork": {"debounceActive": false, "queuedBatches": 0},
                "freshness": "current"
            }
        })
        .to_string()
    }

    #[test]
    fn await_exact_parses_one_consistent_observation() {
        let (path, handle) = serving_socket(await_result());
        let mut client = ControlClient::connect(&path).expect("connect");
        let result = client
            .await_generation(AwaitMode::Exact(7), 100)
            .expect("await");
        handle.join().unwrap();
        let _ = std::fs::remove_file(&path);

        assert_eq!(result.terminal_reason, "passed");
        assert_eq!(result.latest_generation, 7);
        assert_eq!(result.latest_batch, Some(3));
        assert_eq!(result.freshness, "current");
        assert_eq!(result.snapshot.generation, 7);
        assert_eq!(result.snapshot.state, "passed");
        assert!(!result.pending_work.debounce_active);
    }

    #[test]
    fn await_after_sends_after_and_timeout_params() {
        let (path, handle) = serving_socket(await_result());
        let mut client = ControlClient::connect(&path).expect("connect");
        let _ = client
            .await_generation(AwaitMode::After(0), 250)
            .expect("await");
        handle.join().unwrap();
        let _ = std::fs::remove_file(&path);
    }

    #[test]
    fn await_rejects_unknown_terminal_reason() {
        let mut response = serde_json::from_str::<serde_json::Value>(&await_result()).unwrap();
        response["result"]["terminalReason"] = serde_json::json!("hung");
        let (path, handle) = serving_socket(response.to_string());
        let mut client = ControlClient::connect(&path).expect("connect");
        let err = client
            .await_generation(AwaitMode::Exact(7), 100)
            .expect_err("invalid terminal reason must fail closed");
        handle.join().unwrap();
        let _ = std::fs::remove_file(&path);
        assert!(
            matches!(err, ControlClientError::Malformed(_)),
            "unexpected: {err:?}"
        );
    }

    #[test]
    fn output_roundtrip_parses_tasks_and_streams() {
        let response = serde_json::json!({
            "jsonrpc": "2.0",
            "id": 1,
            "result": {
                "generation": 7,
                "tasks": [{
                    "id": "my tests",
                    "stdout": {
                        "content": "line one\nline two\n",
                        "lines": 2,
                        "retainedBytes": 18,
                        "observedBytes": 100,
                        "truncated": true
                    },
                    "stderr": null
                }]
            }
        })
        .to_string();
        let (path, handle) = serving_socket(response);
        let mut client = ControlClient::connect(&path).expect("connect");
        let retrieved = client
            .output(7, Some("my tests"), Some("stdout"), Some(80), false)
            .expect("output");
        handle.join().unwrap();
        let _ = std::fs::remove_file(&path);

        assert_eq!(retrieved.generation, 7);
        assert_eq!(retrieved.tasks.len(), 1);
        let task = &retrieved.tasks[0];
        assert_eq!(task.id, "my tests");
        let stdout = task.stdout.as_ref().expect("stdout");
        assert_eq!(stdout.content, "line one\nline two\n");
        assert_eq!(stdout.lines, 2);
        assert_eq!(stdout.retained_bytes, 18);
        assert_eq!(stdout.observed_bytes, 100);
        assert!(stdout.truncated);
        assert!(task.stderr.is_none());
    }

    #[test]
    fn await_parses_failure_evidence_when_present() {
        let mut result = serde_json::from_str::<serde_json::Value>(&await_result()).unwrap();
        result["result"]["failureEvidence"] = serde_json::json!({
            "excerpt": "error: boom\n",
            "lines": 1,
            "truncated": false,
            "totalObservedBytes": 12,
            "retainedBytes": 12,
            "retrieve": "fzz control output --generation 7 --task 'my tests' --tail 80"
        });
        let (path, handle) = serving_socket(result.to_string());
        let mut client = ControlClient::connect(&path).expect("connect");
        let observation = client
            .await_generation(AwaitMode::Exact(7), 100)
            .expect("await");
        handle.join().unwrap();
        let _ = std::fs::remove_file(&path);

        let evidence = observation.failure_evidence.expect("evidence");
        assert_eq!(evidence.excerpt, "error: boom\n");
        assert_eq!(evidence.lines, 1);
        assert_eq!(evidence.total_observed_bytes, 12);
        assert!(evidence.retrieve.contains("--generation 7"));
    }

    #[test]
    fn await_tolerates_absent_failure_evidence() {
        let (path, handle) = serving_socket(await_result());
        let mut client = ControlClient::connect(&path).expect("connect");
        let observation = client
            .await_generation(AwaitMode::Exact(7), 100)
            .expect("await");
        handle.join().unwrap();
        let _ = std::fs::remove_file(&path);
        assert!(observation.failure_evidence.is_none());
    }

    #[test]
    fn capabilities_expose_the_instance_token() {
        let response = serde_json::json!({
            "jsonrpc": "2.0",
            "id": 1,
            "result": {
                "protocolVersion": "1.0",
                "instance": {"token": "fz-abc", "startedAtEpochMs": 1},
                "methods": ["status"]
            }
        })
        .to_string();
        let (path, handle) = serving_socket(response);
        let mut client = ControlClient::connect(&path).expect("connect");
        let caps = client.capabilities().expect("capabilities");
        handle.join().unwrap();
        let _ = std::fs::remove_file(&path);
        assert_eq!(caps.token, "fz-abc");
        assert_eq!(caps.protocol_version, "1.0");
    }

    #[test]
    fn capabilities_roundtrip_parses_the_full_profile() {
        let response = serde_json::json!({
            "jsonrpc": "2.0",
            "id": 1,
            "result": {
                "protocolVersion": "1.0",
                "schemaVersion": 1,
                "watcherVersion": "1.6.0",
                "instance": {"token": "fz-abc", "startedAtEpochMs": 1},
                "methods": ["status", "targets", "run", "cancel"],
                "optionalFields": ["batch", "changed"],
                "outputFormats": ["toon", "json"],
                "limits": {
                    "outputRetentionBytes": 1048576,
                    "maxResponseBytes": 65536,
                    "maxEvidenceLines": 40
                },
                "features": {
                    "atomicAwait": true,
                    "subscription": false,
                    "correlatedSnapshots": false,
                    "outputRetrieval": true,
                    "pendingWork": false
                }
            }
        })
        .to_string();
        let (path, handle) = serving_socket(response);
        let mut client = ControlClient::connect(&path).expect("connect");
        let caps = client.capabilities().expect("capabilities");
        handle.join().unwrap();
        let _ = std::fs::remove_file(&path);

        assert_eq!(caps.token, "fz-abc");
        assert_eq!(caps.protocol_version, "1.0");
        assert_eq!(caps.schema_version, 1);
        assert_eq!(caps.watcher_version, "1.6.0");
        assert_eq!(caps.methods, vec!["status", "targets", "run", "cancel"]);
        assert_eq!(caps.optional_fields, vec!["batch", "changed"]);
        assert_eq!(caps.output_formats, vec!["toon", "json"]);
        assert_eq!(caps.limits.output_retention_bytes, 1_048_576);
        assert_eq!(caps.limits.max_response_bytes, 65_536);
        assert_eq!(caps.limits.max_evidence_lines, 40);
        assert!(caps.features.atomic_await);
        assert!(caps.features.output_retrieval);
        assert!(!caps.features.subscription);
        // TASK-0073: absent on legacy servers, never assumed.
        assert!(!caps.features.sequential_override);
        assert!(caps.has_method("cancel"));
        assert!(!caps.has_method("subscribe"));
    }

    #[test]
    fn capabilities_default_absent_fields_for_legacy_servers() {
        let response = serde_json::json!({
            "jsonrpc": "2.0",
            "id": 1,
            "result": {
                "protocolVersion": "1.0",
                "instance": {"token": "fz-legacy"}
            }
        })
        .to_string();
        let (path, handle) = serving_socket(response);
        let mut client = ControlClient::connect(&path).expect("connect");
        let caps = client.capabilities().expect("capabilities");
        handle.join().unwrap();
        let _ = std::fs::remove_file(&path);
        assert_eq!(caps.token, "fz-legacy");
        assert_eq!(caps.schema_version, 0);
        assert!(caps.watcher_version.is_empty());
        assert!(caps.methods.is_empty());
        assert!(!caps.has_method("cancel"));
        assert_eq!(caps.limits.output_retention_bytes, 0);
        assert!(!caps.features.atomic_await);
    }

    #[test]
    fn request_ids_increment_per_call() {
        // The server echoes the request's own id back; both calls must
        // validate and the client must have used ids 1 then 2.
        let counter = SOCKET_COUNTER.fetch_add(1, Ordering::SeqCst);
        let path = std::env::temp_dir().join(format!(
            "fzz-control-ids-{}-{counter}.sock",
            std::process::id()
        ));
        let _ = std::fs::remove_file(&path);
        let listener = UnixListener::bind(&path).expect("bind test socket");
        let requests = std::sync::Arc::new(std::sync::Mutex::new(Vec::new()));
        let requests_thread = std::sync::Arc::clone(&requests);
        let handle = std::thread::spawn(move || {
            let (stream, _) = listener.accept().expect("accept client");
            let mut reader = BufReader::new(stream.try_clone().expect("clone stream"));
            let mut writer = stream;
            for _ in 0..2 {
                let mut line = String::new();
                reader.read_line(&mut line).expect("read request");
                let request: Value = serde_json::from_str(&line).expect("parse request");
                let id = request["id"].as_u64().expect("request id");
                requests_thread.lock().unwrap().push(id);
                writeln!(
                    writer,
                    "{}",
                    ok_response(id, serde_json::json!({ "generation": id, "state": "idle" }))
                )
                .expect("write response");
            }
        });
        let mut client = ControlClient::connect(&path).expect("connect");
        let first = client.status().expect("first status");
        let second = client.status().expect("second status");
        handle.join().expect("server thread");
        assert_eq!(first.generation, 1);
        assert_eq!(second.generation, 2);
        assert_eq!(*requests.lock().unwrap(), vec![1, 2]);
    }
}
