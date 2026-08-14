//! JSON-RPC 2.0 NDJSON client adapter for the Funzzy control socket.
//!
//! Owns transport and protocol concerns only: connect, NDJSON framing,
//! request IDs, error-object handling, and response validation. CLI
//! presentation never parses protocol shapes; it consumes the typed
//! snapshots produced here. Per TASK-0021 the client is bounded: every
//! read/write carries a timeout and every response is validated against
//! the additive contract in `docs/AGENT-FEEDBACK-CONTRACT.md`.

use serde_json::Value;
use std::fmt;
use std::io::{BufRead, BufReader, Write};
use std::os::unix::net::UnixStream;
use std::path::{Path, PathBuf};
use std::time::Duration;

/// Default bound for a single socket read/write operation.
pub const DEFAULT_IO_TIMEOUT_MS: u64 = 3_000;

/// A client-visible control failure with a deterministic, actionable message.
#[derive(Debug)]
pub enum ControlClientError {
    /// The socket cannot be reached at the given path.
    Unavailable { path: PathBuf, reason: String },
    /// I/O failure while communicating (other than timeout).
    Io(String),
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
        Ok(Self {
            generation,
            state,
            trigger,
            commands,
            duration_ms,
            failures,
        })
    }
}

/// Validated `targets` result entry.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TargetSnapshot {
    pub name: String,
    pub commands: Vec<String>,
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
        Ok(Self { name, commands })
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
    pub fn run(&mut self, target: &str) -> Result<u64, ControlClientError> {
        let result = self.call("run", serde_json::json!({ "target": target }))?;
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
        self.reader.read_line(&mut response).map_err(io_timeout)?;
        if response.trim().is_empty() {
            return Err(ControlClientError::Malformed(
                "server closed the connection without a response".to_string(),
            ));
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
        let generation = client.run("@agent-final").expect("run");
        handle.join().expect("server thread");
        assert_eq!(generation, 7);
    }

    #[test]
    fn run_without_run_id_is_malformed() {
        let result = serde_json::json!({});
        let (path, handle) = serving_socket(ok_response(1, result));
        let mut client = ControlClient::connect(&path).expect("connect");
        let err = client.run("x").expect_err("missing runId must fail");
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
    fn server_error_object_surfaces_code_and_message() {
        let response = serde_json::json!({
            "jsonrpc": "2.0",
            "id": 1,
            "error": {"code": -32000, "message": "No target found for 'nope'"}
        })
        .to_string();
        let (path, handle) = serving_socket(response);
        let mut client = ControlClient::connect(&path).expect("connect");
        let err = client.run("nope").expect_err("server error must surface");
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
