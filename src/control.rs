use crate::awaiting::{AwaitCoordinator, AwaitMode, AwaitResult};
use crate::executor::Event;
use crate::stdout;
use serde_derive::Serialize;
use std::fs;
use std::io::{self, BufRead, BufReader, Read, Write};
use std::os::unix::fs::PermissionsExt;
use std::os::unix::net::{UnixListener, UnixStream};
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};
use std::sync::{Arc, Mutex};
use std::thread::JoinHandle;
use std::time::Duration;

#[derive(Clone, Debug, PartialEq, Serialize)]
#[serde(rename_all = "lowercase")]
pub enum ExecutionState {
    Idle,
    Running,
    Passed,
    Failed,
    Cancelled,
}

#[derive(Clone, Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ControlTarget {
    pub name: String,
    pub commands: Vec<String>,
}

/// One Funzzy process identity (contract §1): the token changes on restart,
/// so pi-watcher can detect instance changes instead of assuming continuity.
#[derive(Clone, Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ControlInstance {
    pub token: String,
    pub started_at_epoch_ms: u64,
}
impl ControlInstance {
    pub fn new() -> Self {
        let started_at_epoch_ms = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map(|duration| duration.as_millis() as u64)
            .unwrap_or(0);
        let nanos = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map(|duration| duration.as_nanos())
            .unwrap_or(0);
        let token = format!("fz-{:016x}{:08x}", nanos, std::process::id());
        Self {
            token,
            started_at_epoch_ms,
        }
    }
}

/// Largest accepted control response; the extension fails closed beyond it.
pub const MAX_RESPONSE_BYTES: u64 = 65_536;
/// Default failure-evidence tail the server emits.
pub const MAX_EVIDENCE_LINES: usize = 40;

#[derive(Clone, Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ControlState {
    generation: u64,
    state: ExecutionState,
    trigger: Option<String>,
    commands: Vec<String>,
    duration_ms: Option<u64>,
    failures: Vec<String>,
    /// Additive correlation fields (contract §1, TASK-0043): legacy fields
    /// above stay verbatim for old clients; these are new keys only.
    /// One state read can never mix generations: all fields are set from the
    /// same event under the same lock.
    batch: Option<u64>,
    changed: Vec<String>,
    predecessor: Option<u64>,
    superseded_by: Option<u64>,
}

impl Default for ControlState {
    fn default() -> Self {
        Self {
            generation: 0,
            state: ExecutionState::Idle,
            trigger: None,
            commands: vec![],
            duration_ms: None,
            failures: vec![],
            batch: None,
            changed: vec![],
            predecessor: None,
            superseded_by: None,
        }
    }
}

impl ControlState {
    /// The latest started generation identity.
    pub fn generation(&self) -> u64 {
        self.generation
    }

    /// The latest execution state.
    pub fn state(&self) -> &ExecutionState {
        &self.state
    }

    /// The superseded-by relation of the latest generation, when replaced.
    pub fn superseded_by(&self) -> Option<u64> {
        self.superseded_by
    }

    pub fn apply(&mut self, event: Event) {
        match event {
            Event::Started {
                run_id,
                trigger,
                batch,
                predecessor,
                changed,
                commands,
            } => {
                self.generation = run_id;
                self.state = ExecutionState::Running;
                self.trigger = Some(trigger);
                self.batch = batch;
                self.changed = changed;
                self.predecessor = predecessor;
                self.superseded_by = None;
                self.commands = commands;
                self.duration_ms = None;
                self.failures.clear();
            }
            Event::Finished {
                superseded_by,
                elapsed,
                failures,
                ..
            } => {
                self.state = if failures.is_empty() {
                    ExecutionState::Passed
                } else {
                    ExecutionState::Failed
                };
                self.duration_ms = Some(elapsed.as_millis() as u64);
                self.failures = failures;
                self.superseded_by = superseded_by;
            }
            Event::Cancelled { superseded_by, .. } => {
                self.state = ExecutionState::Cancelled;
                self.duration_ms = None;
                self.superseded_by = superseded_by;
            }
            Event::Tick { .. } => {}
        }
    }
}

/// Result of routing one synthetic path change through the shared
/// event-to-run policy (contract §5): matched task names plus the scheduled
/// generation, or an explicit unmatched/ignored outcome with no generation.
#[derive(Clone, Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct EmitOutcome {
    pub matched: Vec<String>,
    pub run_id: Option<u64>,
    pub outcome: String,
}

impl EmitOutcome {
    pub fn scheduled(matched: Vec<String>, run_id: u64) -> Self {
        Self {
            matched,
            run_id: Some(run_id),
            outcome: "scheduled".to_owned(),
        }
    }

    pub fn unmatched() -> Self {
        Self {
            matched: vec![],
            run_id: None,
            outcome: "unmatched".to_owned(),
        }
    }

    pub fn ignored() -> Self {
        Self {
            matched: vec![],
            run_id: None,
            outcome: "ignored".to_owned(),
        }
    }
}

type RunTarget = Arc<dyn Fn(String) -> Result<u64, String> + Send + Sync>;
type EmitPath = Arc<dyn Fn(String) -> Result<EmitOutcome, String> + Send + Sync>;

/// Bounded concurrent client threads; waiters never starve the accept loop.
const MAX_CLIENT_THREADS: usize = 64;

pub struct ControlServer {
    path: PathBuf,
    stop: Arc<AtomicBool>,
    thread: Option<JoinHandle<()>>,
}

impl ControlServer {
    #[allow(dead_code)]
    pub fn start(path: &Path, state: Arc<Mutex<ControlState>>) -> io::Result<Self> {
        Self::start_internal(path, state, vec![], None, None, None)
    }

    pub fn start_with_runner<F>(
        path: &Path,
        state: Arc<Mutex<ControlState>>,
        targets: Vec<ControlTarget>,
        run_target: F,
    ) -> io::Result<Self>
    where
        F: Fn(String) -> Result<u64, String> + Send + Sync + 'static,
    {
        Self::start_internal(path, state, targets, Some(Arc::new(run_target)), None, None)
    }

    /// Extends the runner surface with the `emit` method (TASK-0022): the
    /// handler routes one synthetic path through the shared event-to-run
    /// policy and returns matched tasks plus run identity or an explicit
    /// unmatched/ignored outcome.
    pub fn start_with_emit<F, E>(
        path: &Path,
        state: Arc<Mutex<ControlState>>,
        targets: Vec<ControlTarget>,
        run_target: F,
        emit_path: E,
    ) -> io::Result<Self>
    where
        F: Fn(String) -> Result<u64, String> + Send + Sync + 'static,
        E: Fn(String) -> Result<EmitOutcome, String> + Send + Sync + 'static,
    {
        Self::start_with_coordinator(path, state, targets, run_target, emit_path, None)
    }

    /// Extends the surface with the atomic `await` coordinator (TASK-0044):
    /// the `await` method observes and waits under one lock, returns one
    /// consistent snapshot plus terminal reason and freshness, and never
    /// blocks the watcher's scheduling.
    pub fn start_with_coordinator<F, E>(
        path: &Path,
        state: Arc<Mutex<ControlState>>,
        targets: Vec<ControlTarget>,
        run_target: F,
        emit_path: E,
        coordinator: Option<Arc<AwaitCoordinator>>,
    ) -> io::Result<Self>
    where
        F: Fn(String) -> Result<u64, String> + Send + Sync + 'static,
        E: Fn(String) -> Result<EmitOutcome, String> + Send + Sync + 'static,
    {
        Self::start_internal(
            path,
            state,
            targets,
            Some(Arc::new(run_target)),
            Some(Arc::new(emit_path)),
            coordinator,
        )
    }

    fn start_internal(
        path: &Path,
        state: Arc<Mutex<ControlState>>,
        targets: Vec<ControlTarget>,
        run_target: Option<RunTarget>,
        emit_path: Option<EmitPath>,
        coordinator: Option<Arc<AwaitCoordinator>>,
    ) -> io::Result<Self> {
        prepare_socket_path(path)?;
        let listener = UnixListener::bind(path)?;
        fs::set_permissions(path, fs::Permissions::from_mode(0o600))?;
        listener.set_nonblocking(true)?;

        let stop = Arc::new(AtomicBool::new(false));
        let thread_stop = Arc::clone(&stop);
        let instance = ControlInstance::new();
        let active_clients = Arc::new(AtomicUsize::new(0));
        let thread = std::thread::spawn(move || {
            while !thread_stop.load(Ordering::Relaxed) {
                match listener.accept() {
                    Ok((stream, _)) => {
                        // One thread per client so long `await` waits never
                        // block other clients or the accept loop (TASK-0044).
                        let client_count = active_clients.fetch_add(1, Ordering::Relaxed) + 1;
                        if client_count > MAX_CLIENT_THREADS {
                            active_clients.fetch_sub(1, Ordering::Relaxed);
                            let _ = write_response_ref(
                                &stream,
                                rpc_error(
                                    serde_json::Value::Null,
                                    -32000,
                                    "Server error",
                                    Some(serde_json::json!("too many concurrent control clients")),
                                ),
                            );
                            continue;
                        }
                        let state = Arc::clone(&state);
                        let targets = targets.clone();
                        let run_target = run_target.clone();
                        let emit_path = emit_path.clone();
                        let coordinator = coordinator.clone();
                        let instance = instance.clone();
                        let clients = Arc::clone(&active_clients);
                        std::thread::spawn(move || {
                            handle_client(
                                stream,
                                &state,
                                &targets,
                                run_target.as_ref(),
                                emit_path.as_ref(),
                                coordinator.as_ref(),
                                &instance,
                            );
                            clients.fetch_sub(1, Ordering::Relaxed);
                        });
                    }
                    Err(err) if err.kind() == io::ErrorKind::WouldBlock => {
                        std::thread::sleep(Duration::from_millis(20));
                    }
                    Err(err)
                        if matches!(
                            err.raw_os_error(),
                            Some(
                                nix::libc::EMFILE
                                    | nix::libc::ENFILE
                                    | nix::libc::ENOBUFS
                                    | nix::libc::ENOMEM
                            )
                        ) =>
                    {
                        // Resource exhaustion under load is transient: fd/pipe
                        // pressure must never kill the control surface. Back
                        // off briefly and keep accepting.
                        stdout::warn(&format!(
                            "Control socket accept backed off on resource pressure: {}",
                            err
                        ));
                        std::thread::sleep(Duration::from_millis(200));
                    }
                    Err(err) => {
                        stdout::error(&format!(
                            "Control socket stopped accepting clients: {}",
                            err
                        ));
                        break;
                    }
                }
            }
        });

        Ok(Self {
            path: path.to_path_buf(),
            stop,
            thread: Some(thread),
        })
    }
}

impl Drop for ControlServer {
    fn drop(&mut self) {
        self.stop.store(true, Ordering::Relaxed);
        if let Some(thread) = self.thread.take() {
            let _ = thread.join();
        }
        let _ = fs::remove_file(&self.path);
    }
}

fn prepare_socket_path(path: &Path) -> io::Result<()> {
    if let Some(parent) = path.parent() {
        if !parent.as_os_str().is_empty() {
            fs::create_dir_all(parent)?;
        }
    }

    if !path.exists() {
        return Ok(());
    }

    if UnixStream::connect(path).is_ok() {
        return Err(io::Error::new(
            io::ErrorKind::AddrInUse,
            format!("control socket is already in use: {}", path.display()),
        ));
    }

    fs::remove_file(path)
}

fn handle_client(
    mut stream: UnixStream,
    state: &Arc<Mutex<ControlState>>,
    targets: &[ControlTarget],
    run_target: Option<&RunTarget>,
    emit_path: Option<&EmitPath>,
    coordinator: Option<&Arc<AwaitCoordinator>>,
    instance: &ControlInstance,
) {
    // One NDJSON connection serves multiple requests (JSON-RPC over the
    // socket): the client adapter keeps one connection and increments ids.
    // `await` may hold the connection for its whole bound; per-connection
    // threads keep other clients unblocked.
    //
    // Accepted streams inherit O_NONBLOCK from the nonblocking listener on
    // macOS, so restore blocking reads first; `handle_await` temporarily
    // re-enables nonblocking for disconnect detection and restores it.
    if let Err(err) = stream.set_nonblocking(false) {
        stdout::error(&format!(
            "Control client stream could not be made blocking: {}",
            err
        ));
        return;
    }
    loop {
        let mut request = String::new();
        match BufReader::new(&stream).read_line(&mut request) {
            Ok(0) | Err(_) => return,
            Ok(_) => {}
        }

        let request: serde_json::Value = match serde_json::from_str(&request) {
            Ok(request) => request,
            Err(error) => {
                write_response(
                    &mut stream,
                    rpc_error(
                        serde_json::Value::Null,
                        -32700,
                        "Parse error",
                        Some(serde_json::json!(error.to_string())),
                    ),
                );
                return;
            }
        };

        // `await` blocks for up to its timeout, so it needs the live stream
        // for disconnect detection and is handled outside the dispatcher; it
        // restores blocking mode before returning to the loop.
        if request.get("method").and_then(serde_json::Value::as_str) == Some("await")
            && request
                .get("params")
                .is_some_and(|params| params.is_object())
        {
            handle_await(&mut stream, request, state, coordinator);
            continue;
        }

        if let Some(response) =
            process_payload(request, state, targets, run_target, emit_path, instance)
        {
            write_response(&mut stream, response);
        }
    }
}

/// Validates one `await` request (contract §4): exactly one of `after` /
/// `generation` plus a positive `timeoutMs`. Waits with the shared atomic
/// primitive, probing the socket between wake slices to free waiters whose
/// client disconnected. Timeouts perform no cancellation.
fn handle_await(
    stream: &mut UnixStream,
    request: serde_json::Value,
    state: &Arc<Mutex<ControlState>>,
    coordinator: Option<&Arc<AwaitCoordinator>>,
) {
    let id = request_id(&request);
    if request.get("jsonrpc") != Some(&serde_json::json!("2.0")) {
        write_response(stream, rpc_error(id, -32600, "Invalid Request", None));
        return;
    }
    let Some(coordinator) = coordinator else {
        write_response(
            stream,
            rpc_error(
                id,
                -32000,
                "Server error",
                Some(serde_json::json!("await is unavailable")),
            ),
        );
        return;
    };

    let params = match await_params(&request) {
        Ok(params) => params,
        Err(data) => {
            write_response(stream, rpc_error(id, -32602, "Invalid params", Some(data)));
            return;
        }
    };

    // Nonblocking so the waiter can detect client disconnect between slices.
    let _ = stream.set_nonblocking(true);
    let mut probe = || {
        let mut buffer = [0u8; 16];
        match stream.read(&mut buffer) {
            Ok(0) => true,
            Ok(_) => true,
            Err(err) if err.kind() == io::ErrorKind::WouldBlock => false,
            Err(_) => true,
        }
    };
    let result: AwaitResult =
        coordinator.await_generation(params.mode, params.timeout, state, Some(&mut probe));
    let _ = stream.set_nonblocking(false);
    write_response(
        stream,
        serde_json::json!({
            "jsonrpc": "2.0",
            "id": id,
            "result": serde_json::to_value(result).unwrap_or(serde_json::Value::Null),
        }),
    );
}

struct AwaitParams {
    mode: AwaitMode,
    timeout: Duration,
}

fn await_params(request: &serde_json::Value) -> Result<AwaitParams, serde_json::Value> {
    let params = request.get("params").and_then(serde_json::Value::as_object);
    let after = params
        .and_then(|params| params.get("after"))
        .and_then(serde_json::Value::as_u64);
    let generation = params
        .and_then(|params| params.get("generation"))
        .and_then(serde_json::Value::as_u64);
    let timeout_ms = params
        .and_then(|params| params.get("timeoutMs"))
        .and_then(serde_json::Value::as_u64)
        .filter(|timeout| *timeout > 0);

    match (after, generation, timeout_ms) {
        (Some(after), None, Some(timeout)) => Ok(AwaitParams {
            mode: AwaitMode::After(after),
            timeout: Duration::from_millis(timeout),
        }),
        (None, Some(generation), Some(timeout)) => Ok(AwaitParams {
            mode: AwaitMode::Exact(generation),
            timeout: Duration::from_millis(timeout),
        }),
        (Some(_), Some(_), _) => Err(serde_json::json!(
            "await requires exactly one of params.after or params.generation"
        )),
        (_, _, None) => Err(serde_json::json!(
            "await requires a positive numeric params.timeoutMs"
        )),
        _ => Err(serde_json::json!(
            "await requires exactly one of params.after or params.generation and a positive params.timeoutMs"
        )),
    }
}

fn write_response_ref(stream: &UnixStream, response: serde_json::Value) -> io::Result<()> {
    use std::io::Write as _;
    let mut stream = stream.try_clone()?;
    writeln!(stream, "{}", response)
}

fn process_payload(
    request: serde_json::Value,
    state: &Arc<Mutex<ControlState>>,
    targets: &[ControlTarget],
    run_target: Option<&RunTarget>,
    emit_path: Option<&EmitPath>,
    instance: &ControlInstance,
) -> Option<serde_json::Value> {
    let serde_json::Value::Array(requests) = request else {
        return process_request(request, state, targets, run_target, emit_path, instance);
    };

    if requests.is_empty() {
        return Some(rpc_error(
            serde_json::Value::Null,
            -32600,
            "Invalid Request",
            None,
        ));
    }

    let responses: Vec<_> = requests
        .into_iter()
        .filter_map(|request| {
            process_request(request, state, targets, run_target, emit_path, instance)
        })
        .collect();
    if responses.is_empty() {
        return None;
    }
    Some(serde_json::Value::Array(responses))
}

fn process_request(
    request: serde_json::Value,
    state: &Arc<Mutex<ControlState>>,
    targets: &[ControlTarget],
    run_target: Option<&RunTarget>,
    emit_path: Option<&EmitPath>,
    instance: &ControlInstance,
) -> Option<serde_json::Value> {
    let id = request_id(&request);
    let Some(object) = request.as_object() else {
        return Some(rpc_error(id, -32600, "Invalid Request", None));
    };
    let Some(method) = object.get("method").and_then(|method| method.as_str()) else {
        return Some(rpc_error(id, -32600, "Invalid Request", None));
    };
    if object.get("jsonrpc") != Some(&serde_json::json!("2.0")) {
        return Some(rpc_error(id, -32600, "Invalid Request", None));
    }
    if object
        .get("params")
        .is_some_and(|params| !params.is_object() && !params.is_array())
    {
        return Some(rpc_error(id, -32602, "Invalid params", None));
    }

    let result = match method {
        "status" => Ok(serde_json::json!(state.lock().unwrap().clone())),
        "targets" => Ok(serde_json::json!(targets)),
        "run" => run_requested_target(&request, run_target),
        "emit" => emit_requested_path(&request, emit_path),
        // Honest negotiated profile (contract §8): methods list only what this
        // server implements; features stay false until the additive contract
        // (subscribe, cancel, output, correlated snapshots) lands. The
        // extension keeps the legacy polling fallback and never assumes
        // capabilities from package versions.
        "capabilities" => Ok(serde_json::json!({
            "protocolVersion": "1.0",
            "schemaVersion": 1,
            "instance": {
                "token": instance.token,
                "startedAtEpochMs": instance.started_at_epoch_ms,
            },
            "methods": ["status", "targets", "run", "emit", "await", "capabilities"],
            "optionalFields": [],
            "limits": {
                "outputRetentionBytes": 0,
                "maxResponseBytes": MAX_RESPONSE_BYTES,
                "maxEvidenceLines": MAX_EVIDENCE_LINES,
            },
            "features": {
                "atomicAwait": true,
                "subscription": false,
                "correlatedSnapshots": false,
                "outputRetrieval": false,
                "pendingWork": false,
            },
        })),
        _ => Err((-32601, "Method not found", None)),
    };

    // Wire contract: the pi-watcher extension decodes these results from
    // `unknown` (pi-watcher/src/domain/watcher.ts) and fails closed on any
    // shape drift. Change the serializers here and the decoders together.
    //
    // Additive contract: pi-watcher negotiates `capabilities` and correlated
    // snapshots (pi-watcher/src/domain/capabilities.ts). Golden wire fixtures
    // live in pi-watcher/src/domain/fixtures/*.json and MUST stay in sync with
    // any Rust test that asserts these payloads — change both together.

    if !object.contains_key("id") {
        return None;
    }
    Some(match result {
        Ok(result) => serde_json::json!({"jsonrpc": "2.0", "id": id, "result": result}),
        Err((code, message, data)) => rpc_error(id, code, message, data),
    })
}

fn request_id(request: &serde_json::Value) -> serde_json::Value {
    request
        .get("id")
        .filter(|id| id.is_null() || id.is_string() || id.is_number())
        .cloned()
        .unwrap_or(serde_json::Value::Null)
}

fn run_requested_target(
    request: &serde_json::Value,
    run_target: Option<&RunTarget>,
) -> Result<serde_json::Value, (i64, &'static str, Option<serde_json::Value>)> {
    let Some(target) = request
        .get("params")
        .and_then(|params| params.get("target"))
        .and_then(|target| target.as_str())
        .filter(|target| !target.trim().is_empty())
    else {
        return Err((
            -32602,
            "Invalid params",
            Some(serde_json::json!("run requires a non-empty params.target")),
        ));
    };
    let Some(run_target) = run_target else {
        return Err((
            -32000,
            "Server error",
            Some(serde_json::json!("target execution is unavailable")),
        ));
    };

    run_target(target.to_string())
        .map(|run_id| serde_json::json!({"runId": run_id}))
        .map_err(|error| (-32000, "Server error", Some(serde_json::json!(error))))
}

/// `emit` transport validation only: one non-empty path. Routing, matching,
/// ignore precedence, ordering, templates, and busy-run policy stay in the
/// injected handler (shared event-to-run policy); no second matcher or
/// executor lives in `control.rs`.
fn emit_requested_path(
    request: &serde_json::Value,
    emit_path: Option<&EmitPath>,
) -> Result<serde_json::Value, (i64, &'static str, Option<serde_json::Value>)> {
    let Some(path) = request
        .get("params")
        .and_then(|params| params.get("path"))
        .and_then(|path| path.as_str())
        .filter(|path| !path.trim().is_empty())
        .filter(|path| !path.contains('\0'))
    else {
        return Err((
            -32602,
            "Invalid params",
            Some(serde_json::json!(
                "emit requires a non-empty params.path without NUL bytes"
            )),
        ));
    };
    let Some(emit_path) = emit_path else {
        return Err((
            -32000,
            "Server error",
            Some(serde_json::json!("path emission is unavailable")),
        ));
    };

    emit_path(path.to_string())
        .map(|outcome| serde_json::to_value(outcome).unwrap_or(serde_json::Value::Null))
        .map_err(|error| (-32000, "Server error", Some(serde_json::json!(error))))
}

fn rpc_error(
    id: serde_json::Value,
    code: i64,
    message: &str,
    data: Option<serde_json::Value>,
) -> serde_json::Value {
    let mut error = serde_json::json!({"code": code, "message": message});
    if let Some(data) = data {
        error["data"] = data;
    }
    serde_json::json!({"jsonrpc": "2.0", "id": id, "error": error})
}

fn write_response(stream: &mut UnixStream, response: serde_json::Value) {
    let _ = writeln!(stream, "{}", response);
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::executor::Event;
    use std::time::Duration;

    fn started(run_id: u64, batch: Option<u64>, predecessor: Option<u64>) -> Event {
        Event::Started {
            run_id,
            trigger: "src/main.rs".to_owned(),
            batch,
            predecessor,
            changed: vec!["src/main.rs".to_owned()],
            commands: vec!["echo hi".to_owned()],
        }
    }

    #[test]
    fn started_sets_all_correlation_fields_coherently() {
        let mut state = ControlState::default();
        state.apply(started(42, Some(7), Some(41)));

        assert_eq!(state.generation, 42);
        assert_eq!(state.state, ExecutionState::Running);
        assert_eq!(state.trigger.as_deref(), Some("src/main.rs"));
        assert_eq!(state.batch, Some(7));
        assert_eq!(state.changed, vec!["src/main.rs".to_owned()]);
        assert_eq!(state.predecessor, Some(41));
        assert_eq!(state.superseded_by, None);
        assert_eq!(state.duration_ms, None);
        assert!(state.failures.is_empty());
    }

    #[test]
    fn finished_records_terminal_state_and_superseded_relation() {
        let mut state = ControlState::default();
        state.apply(started(42, None, None));
        state.apply(Event::Finished {
            run_id: 42,
            superseded_by: Some(43),
            elapsed: Duration::from_millis(9),
            failures: vec!["boom".to_owned()],
        });

        assert_eq!(state.state, ExecutionState::Failed);
        assert_eq!(state.duration_ms, Some(9));
        assert_eq!(state.failures, vec!["boom".to_owned()]);
        assert_eq!(state.superseded_by, Some(43));
        // Correlation fields still describe generation 42, never a mix.
        assert_eq!(state.generation, 42);
    }

    #[test]
    fn cancelled_records_generation_and_superseded_by() {
        let mut state = ControlState::default();
        state.apply(started(1, None, None));
        state.apply(Event::Cancelled {
            run_id: 1,
            superseded_by: Some(2),
        });

        assert_eq!(state.state, ExecutionState::Cancelled);
        assert_eq!(state.superseded_by, Some(2));
        assert_eq!(state.duration_ms, None);
        assert_eq!(state.generation, 1);
    }

    #[test]
    fn replacement_transition_keeps_latest_generation_consistent() {
        let mut state = ControlState::default();
        state.apply(started(1, None, None));
        state.apply(Event::Cancelled {
            run_id: 1,
            superseded_by: Some(2),
        });
        state.apply(started(2, Some(5), Some(1)));

        // One state read: generation 2, its batch, and its predecessor —
        // never a mixture of the superseded generation's fields.
        assert_eq!(state.generation, 2);
        assert_eq!(state.batch, Some(5));
        assert_eq!(state.predecessor, Some(1));
        assert_eq!(state.superseded_by, None);
        assert_eq!(state.state, ExecutionState::Running);
    }

    #[test]
    fn legacy_fields_serialize_verbatim_with_additive_correlation_keys() {
        let mut state = ControlState::default();
        state.apply(started(42, Some(7), None));
        let json = serde_json::to_value(state.clone()).unwrap();
        let object = json.as_object().unwrap();

        // Legacy keys preserved.
        assert_eq!(object["generation"], serde_json::json!(42));
        assert_eq!(object["state"], serde_json::json!("running"));
        assert_eq!(object["trigger"], serde_json::json!("src/main.rs"));
        assert!(object.contains_key("commands"));
        assert!(object.contains_key("durationMs"));
        assert!(object.contains_key("failures"));

        // Additive correlation keys, camelCase.
        assert_eq!(object["batch"], serde_json::json!(7));
        assert_eq!(object["changed"], serde_json::json!(["src/main.rs"]));
        assert_eq!(object["predecessor"], serde_json::json!(null));
        assert_eq!(object["supersededBy"], serde_json::json!(null));
    }
}

#[cfg(test)]
mod loop_tests {
    use super::*;
    use std::io::Write;

    #[test]
    fn one_connection_serves_multiple_requests() {
        let path = std::env::temp_dir().join(format!(
            "fzz-loop-{}-{}.sock",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ));
        let state = Arc::new(Mutex::new(ControlState::default()));
        let _server = ControlServer::start(&path, Arc::clone(&state)).unwrap();

        let mut stream = UnixStream::connect(&path).expect("connect");
        stream
            .set_read_timeout(Some(Duration::from_secs(3)))
            .expect("read timeout");
        let request = r#"{"jsonrpc":"2.0","id":1,"method":"status","params":{}}"#;
        for id in 1..=2 {
            writeln!(
                stream,
                "{}",
                request.replace("\"id\":1", &format!("\"id\":{}", id))
            )
            .expect("write request");
            let mut line = String::new();
            BufReader::new(&stream)
                .read_line(&mut line)
                .expect("read response");
            let response: serde_json::Value = serde_json::from_str(&line).expect("parse");
            assert_eq!(response["id"], id, "response for request {id}: {line}");
        }
    }
}
