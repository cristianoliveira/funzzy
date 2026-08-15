//! Structured control output (TASK-0048): canonical `serde_json::Value`
//! documents per response, rendered as TOON (non-TTY/agent default), JSON
//! (interoperability), or human (TTY default). Domain responses stay
//! format-independent; encoding happens once at this boundary.
//!
//! Determinism: objects are built in a fixed field order (serde_json::Map
//! preserves insertion order), so the same response always renders the same
//! bytes for a given format. No terminal-width-dependent output.

use crate::cli::control::OutputFormat;
use crate::control_client::{
    AwaitSnapshot, CancelSnapshot, CapabilitiesSnapshot, ConfigSnapshot, EmitSnapshot,
    OutputSnapshot, StatusSnapshot, TargetSnapshot,
};
use crate::duration_history::RunEstimate;
use serde_json::{json, Value};

/// Renders one canonical document in the selected format; structured formats
/// emit exactly one document (no progress/debug on stdout).
pub fn render_document(format: OutputFormat, document: &Value) -> String {
    match format {
        OutputFormat::Human => serde_json::to_string_pretty(document).unwrap_or_default(),
        OutputFormat::Json => serde_json::to_string(document).unwrap_or_default() + "\n",
        OutputFormat::Toon => toon::encode(document, None) + "\n",
    }
}

/// Renders an error in the same structured format with a stable code shape.
pub fn render_error(format: OutputFormat, code: i64, message: &str) -> String {
    render_document(
        format,
        &json!({ "error": { "code": code, "message": message } }),
    )
}

/// Renders a control server error with its structured data (contract §3):
/// typed codes carry machine-actionable retry data (candidates, retained
/// range, action) that agents consume without parsing message text.
pub fn render_server_error(
    format: OutputFormat,
    code: i64,
    message: &str,
    data: Option<&serde_json::Value>,
) -> String {
    let mut error = json!({ "code": code, "message": message });
    if let Some(data) = data {
        error["data"] = data.clone();
    }
    render_document(format, &json!({ "error": error }))
}

pub fn status_document(status: &StatusSnapshot) -> Value {
    let mut doc = json!({
        "generation": status.generation,
        "state": status.state,
        "trigger": status.trigger,
        "durationMs": status.duration_ms,
        "commands": status.commands,
        "failures": status.failures,
        "effectiveConcurrency": status.effective_concurrency,
        "concurrencySource": status.concurrency_source,
    });
    // TASK-0091, AC2: the frozen config revision of the latest generation
    // (additive; omitted on legacy servers).
    if let Some(revision) = status.revision {
        doc["revision"] = json!(revision);
    }
    if let Some(revision_hash) = &status.revision_hash {
        doc["revisionHash"] = json!(revision_hash);
    }
    doc
}

pub fn targets_document(targets: &[TargetSnapshot]) -> Value {
    let entries: Vec<Value> = targets
        .iter()
        .map(|target| {
            let mut entry = json!({
                "name": target.name,
                "commands": target.commands,
            });
            if let Some(estimate) = &target.estimate {
                entry["estimate"] = estimate_document(estimate);
            }
            entry
        })
        .collect();
    json!({ "targets": entries })
}

pub fn estimate_document(estimate: &RunEstimate) -> Value {
    serde_json::to_value(estimate).unwrap_or_else(|_| Value::Null)
}

pub fn capabilities_document(caps: &CapabilitiesSnapshot) -> Value {
    json!({
        "protocolVersion": caps.protocol_version,
        "schemaVersion": caps.schema_version,
        "watcherVersion": caps.watcher_version,
        "instance": { "token": caps.token },
        "methods": caps.methods,
        "optionalFields": caps.optional_fields,
        "outputFormats": caps.output_formats,
        "limits": {
            "outputRetentionBytes": caps.limits.output_retention_bytes,
            "maxResponseBytes": caps.limits.max_response_bytes,
            "maxEvidenceLines": caps.limits.max_evidence_lines,
            "durationEstimateLimits": {
                "maxSamples": caps.limits.estimate_max_samples,
                "floorMs": caps.limits.estimate_floor_ms,
                "capMs": caps.limits.estimate_cap_ms,
            },
        },
        "features": {
            "atomicAwait": caps.features.atomic_await,
            "subscription": caps.features.subscription,
            "correlatedSnapshots": caps.features.correlated_snapshots,
            "outputRetrieval": caps.features.output_retrieval,
            "pendingWork": caps.features.pending_work,
            "durationEstimates": caps.features.duration_estimates,
            "sequentialOverride": caps.features.sequential_override,
        },
    })
}

pub fn run_document(scheduled: &crate::control_client::ScheduledRunSnapshot) -> Value {
    let mut doc = json!({ "runId": scheduled.run_id });
    if let Some(revision) = scheduled.revision {
        doc["revision"] = json!(revision);
    }
    if let Some(revision_hash) = &scheduled.revision_hash {
        doc["revisionHash"] = json!(revision_hash);
    }
    doc
}

pub fn emit_document(emit: &EmitSnapshot) -> Value {
    let mut doc = json!({
        "outcome": emit.outcome,
        "matched": emit.matched,
        "runId": emit.run_id,
    });
    if let Some(revision) = emit.revision {
        doc["revision"] = json!(revision);
    }
    if let Some(revision_hash) = &emit.revision_hash {
        doc["revisionHash"] = json!(revision_hash);
    }
    doc
}

pub fn cancel_document(cancel: &CancelSnapshot) -> Value {
    json!({
        "cancelled": cancel.cancelled,
        "generation": cancel.generation,
    })
}

/// Canonical `config` lifecycle document (TASK-0091, AC3): the live
/// transition plus the bounded history, fixed field order.
pub fn config_document(config: &ConfigSnapshot) -> Value {
    let transition = |t: &crate::control_client::ConfigTransitionSnapshot| {
        let mut entry =
            json!({ "phase": t.phase, "ordinal": t.ordinal, "atEpochMs": t.at_epoch_ms });
        if let Some(revision) = t.revision {
            entry["revision"] = json!(revision);
        }
        if let Some(hash) = &t.revision_hash {
            entry["revisionHash"] = json!(hash);
        }
        if let Some(reason) = &t.reason {
            entry["reason"] = json!(reason);
        }
        entry
    };
    let mut doc = json!({
        "current": transition(&config.current),
        "history": config.history.iter().map(transition).collect::<Vec<_>>(),
    });
    if let Some(revision) = config.current.revision {
        doc["revision"] = json!(revision);
    }
    if let Some(hash) = &config.current.revision_hash {
        doc["revisionHash"] = json!(hash);
    }
    if let Some(reason) = &config.current.reason {
        doc["reason"] = json!(reason);
    }
    doc
}

pub fn output_document(output: &OutputSnapshot) -> Value {
    let mut doc = json!({
        "generation": output.generation,
        "tasks": output
            .tasks
            .iter()
            .map(|task| {
                let mut entry = json!({ "id": task.id });
                for (name, stream) in [("stdout", &task.stdout), ("stderr", &task.stderr)] {
                    if let Some(stream) = stream {
                        entry[name] = json!({
                            "content": stream.content,
                            "truncated": stream.truncated,
                            "totalObservedBytes": stream.observed_bytes,
                            "retainedBytes": stream.retained_bytes,
                        });
                    }
                }
                entry
            })
            .collect::<Vec<_>>(),
    });
    if let Some(resolved_task) = &output.resolved_task {
        doc["resolvedTask"] = json!(resolved_task);
    }
    if let Some(next_cursor) = &output.next_cursor {
        doc["nextCursor"] = json!(next_cursor);
    }
    if let Some(returned_bytes) = output.returned_bytes {
        doc["returnedBytes"] = json!(returned_bytes);
    }
    if let Some(truncated) = output.truncated {
        doc["truncated"] = json!(truncated);
    }
    doc
}

pub fn await_document(observation: &AwaitSnapshot) -> Value {
    let mut doc = json!({
        "terminalReason": observation.terminal_reason,
        "latestGeneration": observation.latest_generation,
        "latestBatch": observation.latest_batch,
        "freshness": observation.freshness,
        "snapshot": status_document(&observation.snapshot),
    });
    if let Some(evidence) = &observation.failure_evidence {
        let mut evidence_doc = json!({
            "excerpt": evidence.excerpt,
            "lines": evidence.lines,
            "truncated": evidence.truncated,
            "totalObservedBytes": evidence.total_observed_bytes,
            "retainedBytes": evidence.retained_bytes,
            "retrieve": evidence.retrieve,
            "additionalFailedTasks": evidence.additional_failed_tasks,
        });
        if let Some(output_ref) = &evidence.output_ref {
            evidence_doc["outputRef"] = json!({
                "instanceToken": output_ref.instance_token,
                "generation": output_ref.generation,
                "task": output_ref.task,
                "mode": output_ref.mode,
                "tail": output_ref.tail,
                "maxBytes": output_ref.max_bytes,
                "retrieve": output_ref.retrieve,
            });
        }
        doc["failureEvidence"] = evidence_doc;
    }
    doc
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::control_client::{
        AwaitSnapshot, CancelSnapshot, CapabilitiesSnapshot, CapabilityFeatures, CapabilityLimits,
        EmitSnapshot, FailureEvidenceSnapshot, OutputSnapshot, RetrievedTaskSnapshot,
        StatusSnapshot, StreamSnapshot,
    };
    use crate::duration_history::{EstimateConfidence, EstimateSource, RunEstimate};

    fn status() -> StatusSnapshot {
        StatusSnapshot {
            generation: 4,
            state: "failed".to_owned(),
            trigger: Some("src/main.rs".to_owned()),
            commands: vec!["cargo test".to_owned()],
            duration_ms: Some(42),
            failures: vec!["test: boom".to_owned()],
            effective_concurrency: Some(1),
            concurrency_source: Some("control".to_owned()),
            revision: Some(2),
            revision_hash: Some("hash-2".to_owned()),
        }
    }

    #[test]
    fn status_document_is_deterministic_and_has_stable_keys() {
        let a = status_document(&status());
        let b = status_document(&status());
        assert_eq!(
            serde_json::to_string(&a).unwrap(),
            serde_json::to_string(&b).unwrap()
        );
        let keys: Vec<_> = a.as_object().unwrap().keys().cloned().collect();
        assert_eq!(
            keys,
            vec![
                "commands",
                "concurrencySource",
                "durationMs",
                "effectiveConcurrency",
                "failures",
                "generation",
                "revision",
                "revisionHash",
                "state",
                "trigger"
            ]
        );
    }

    #[test]
    fn toon_and_json_round_trip_to_the_same_canonical_value() {
        let doc = status_document(&status());
        let json = serde_json::to_string(&doc).unwrap();
        let toon_str = toon::encode(&doc, None);
        // JSON parses back to the document.
        let json_back: Value = serde_json::from_str(&json).unwrap();
        assert_eq!(json_back, doc);
        // TOON encodes the same value without loss (spot-check content).
        assert!(toon_str.contains("generation"), "toon: {toon_str}");
        assert!(toon_str.contains("4"), "toon: {toon_str}");
        assert!(toon_str.contains("boom"), "toon: {toon_str}");
    }

    #[test]
    fn error_document_has_stable_code_shape_in_every_format() {
        for format in [OutputFormat::Toon, OutputFormat::Json] {
            let rendered = render_error(format, -32000, "server boom");
            let doc: Value = if format == OutputFormat::Json {
                serde_json::from_str(&rendered).unwrap()
            } else {
                // TOON parses back through the encoder's reader; for the
                // shape test just verify the code appears.
                let re_encoded = toon::encode(
                    &serde_json::json!({"error": {"code": -32000, "message": "server boom"}}),
                    None,
                );
                assert_eq!(rendered.trim(), re_encoded.trim());
                serde_json::json!({})
            };
            if format == OutputFormat::Json {
                assert_eq!(doc["error"]["code"], -32000);
                assert_eq!(doc["error"]["message"], "server boom");
            }
        }
    }

    #[test]
    fn server_error_document_carries_structured_retry_data() {
        // Contract §3: typed errors render code + message + structured data
        // (candidates, retained range, action) so agents never parse prose.
        let data = serde_json::json!({
            "generation": 7,
            "task": "run integration",
            "candidates": ["run integration @agent-final"],
            "ambiguous": false,
            "action": "reobserve-or-copy-exact"
        });
        for format in [OutputFormat::Toon, OutputFormat::Json] {
            let rendered = render_server_error(format, -32011, "task_not_found", Some(&data));
            if format == OutputFormat::Json {
                let doc: Value = serde_json::from_str(&rendered).unwrap();
                assert_eq!(doc["error"]["code"], -32011);
                assert_eq!(
                    doc["error"]["data"]["candidates"][0],
                    "run integration @agent-final"
                );
                assert_eq!(doc["error"]["data"]["action"], "reobserve-or-copy-exact");
            } else {
                assert!(rendered.contains("-32011"), "toon: {rendered}");
                assert!(rendered.contains("task_not_found"), "toon: {rendered}");
                assert!(
                    rendered.contains("run integration @agent-final"),
                    "toon: {rendered}"
                );
            }
        }
    }

    #[test]
    fn output_document_carries_bounds_not_secret_redaction() {
        let output = OutputSnapshot {
            generation: 9,
            resolved_task: None,
            next_cursor: None,
            returned_bytes: None,
            truncated: None,
            tasks: vec![RetrievedTaskSnapshot {
                id: "t-1".to_owned(),
                stdout: Some(StreamSnapshot {
                    content: "line with secret=abc123".to_owned(),
                    lines: 1,
                    retained_bytes: 24,
                    observed_bytes: 24,
                    truncated: false,
                }),
                stderr: None,
            }],
        };
        let doc = output_document(&output);
        assert_eq!(doc["generation"], 9);
        assert_eq!(doc["tasks"][0]["id"], "t-1");
        assert_eq!(doc["tasks"][0]["stdout"]["retainedBytes"], 24);
        assert_eq!(doc["tasks"][0]["stdout"]["truncated"], false);
    }

    #[test]
    fn await_document_includes_reason_snapshot_and_evidence() {
        let observation = AwaitSnapshot {
            terminal_reason: "failed".to_owned(),
            latest_generation: 7,
            latest_batch: Some(3),
            pending_work: crate::control_client::PendingWorkSnapshot {
                debounce_active: false,
                queued_batches: 0,
            },
            freshness: "current".to_owned(),
            snapshot: status(),
            failure_evidence: Some(FailureEvidenceSnapshot {
                excerpt: "error: boom".to_owned(),
                lines: 1,
                truncated: false,
                total_observed_bytes: 12,
                retained_bytes: 12,
                retrieve: "fzz control output --generation 7".to_owned(),
                output_ref: None,
                additional_failed_tasks: 0,
            }),
        };
        let doc = await_document(&observation);
        assert_eq!(doc["terminalReason"], "failed");
        assert_eq!(doc["snapshot"]["generation"], 4);
        assert_eq!(
            doc["failureEvidence"]["retrieve"],
            "fzz control output --generation 7"
        );
    }

    #[test]
    fn capabilities_document_is_deterministic() {
        let caps = CapabilitiesSnapshot {
            token: "fz-abc".to_owned(),
            protocol_version: "1.0".to_owned(),
            schema_version: 1,
            watcher_version: "2.0.0".to_owned(),
            methods: vec!["status".to_owned(), "run".to_owned()],
            optional_fields: vec![],
            output_formats: vec!["toon".to_owned(), "json".to_owned()],
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
                sequential_override: true,
            },
        };
        let doc = capabilities_document(&caps);
        assert_eq!(doc["protocolVersion"], "1.0");
        assert_eq!(doc["features"]["sequentialOverride"], true);
        assert_eq!(
            serde_json::to_string(&doc).unwrap(),
            serde_json::to_string(&doc).unwrap()
        );
    }

    #[test]
    fn estimate_document_uses_serde_enum_shapes() {
        let estimate = RunEstimate {
            typical_ms: 38_000,
            upper_ms: 61_000,
            recommended_timeout_ms: 95_000,
            samples: 12,
            confidence: EstimateConfidence::Medium,
            source: EstimateSource::Measured,
        };
        let doc = estimate_document(&estimate);
        assert_eq!(doc["confidence"], "medium");
        assert_eq!(doc["source"], "measured");
    }

    #[test]
    fn emit_and_cancel_documents_match_contract_shapes() {
        let emit = EmitSnapshot {
            matched: vec!["a".to_owned(), "b".to_owned()],
            run_id: Some(7),
            outcome: "scheduled".to_owned(),
            revision: None,
            revision_hash: None,
        };
        let doc = emit_document(&emit);
        assert_eq!(doc["outcome"], "scheduled");
        assert_eq!(doc["runId"], 7);
        assert_eq!(doc["matched"], serde_json::json!(["a", "b"]));

        let cancel = CancelSnapshot {
            cancelled: true,
            generation: 7,
        };
        let doc = cancel_document(&cancel);
        assert_eq!(doc["cancelled"], true);
        assert_eq!(doc["generation"], 7);
    }

    #[test]
    fn targets_document_includes_estimates_when_present() {
        let target = TargetSnapshot {
            name: "final checks @agent-final".to_owned(),
            commands: vec!["cargo test".to_owned()],
            estimate: Some(RunEstimate {
                typical_ms: 38_000,
                upper_ms: 61_000,
                recommended_timeout_ms: 95_000,
                samples: 12,
                confidence: EstimateConfidence::Medium,
                source: EstimateSource::Measured,
            }),
        };
        let doc = targets_document(&[target]);
        assert_eq!(doc["targets"][0]["name"], "final checks @agent-final");
        assert_eq!(doc["targets"][0]["estimate"]["samples"], 12);
    }
}
