use crate::awaiting::{AwaitCoordinator, AwaitMode, AwaitResult};
use crate::duration_history::RunEstimate;
use crate::executor::CancelDisposition;
use crate::output::{OutputRegistry, DEFAULT_FAILURE_EVIDENCE_LINES, OUTPUT_RETENTION_BYTES};
use crate::snapshot::SnapshotBroker;
use crate::stdout;
use crate::watcher_state::{WatcherExecutionState, WatcherInstance, WatcherState};
use crate::workers::CancelResult;
use serde::Serialize;
use std::fs;
use std::io::{self, BufRead, BufReader, Read, Write};
use std::os::unix::fs::PermissionsExt;
use std::os::unix::net::{UnixListener, UnixStream};
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};
use std::sync::{Arc, Mutex};
use std::thread::JoinHandle;
use std::time::Duration;

#[derive(Clone, Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ControlTarget {
    pub name: String,
    pub commands: Vec<String>,
}

/// Computes current duration estimate for one target at request time.
pub type TargetEstimateProvider = Arc<dyn Fn(&ControlTarget) -> Option<RunEstimate> + Send + Sync>;

/// Largest accepted control response; extension fails closed beyond it.
pub const MAX_RESPONSE_BYTES: u64 = 65_536;

/// Result of routing one synthetic path change through the shared
/// event-to-run policy (contract §5): matched task names plus the scheduled
/// generation, or an explicit unmatched/ignored outcome with no generation.
/// Additive `revision`/`revisionHash` (TASK-0091, AC2) name the frozen
/// config revision the scheduled generation was planned under.
#[derive(Clone, Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct EmitOutcome {
    pub matched: Vec<String>,
    pub run_id: Option<u64>,
    pub outcome: String,
    /// Immutable config revision the scheduled run was frozen under; omitted
    /// for legacy servers and for unmatched/ignored outcomes.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub revision: Option<u64>,
    /// Non-secret semantic hash of the frozen revision.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub revision_hash: Option<String>,
}

impl EmitOutcome {
    pub fn scheduled(matched: Vec<String>, run_id: u64) -> Self {
        Self {
            matched,
            run_id: Some(run_id),
            outcome: "scheduled".to_owned(),
            revision: None,
            revision_hash: None,
        }
    }

    /// Builds a scheduled outcome with the frozen config revision the run
    /// was planned under (TASK-0091, AC2/AC7).
    pub fn scheduled_at(
        matched: Vec<String>,
        run_id: u64,
        revision: Option<crate::config_revision::ConfigRevision>,
    ) -> Self {
        Self {
            matched,
            run_id: Some(run_id),
            outcome: "scheduled".to_owned(),
            revision: revision.as_ref().map(|r| r.number),
            revision_hash: revision.map(|r| r.hash),
        }
    }

    pub fn unmatched() -> Self {
        Self {
            matched: vec![],
            run_id: None,
            outcome: "unmatched".to_owned(),
            revision: None,
            revision_hash: None,
        }
    }

    pub fn ignored() -> Self {
        Self {
            matched: vec![],
            run_id: None,
            outcome: "ignored".to_owned(),
            revision: None,
            revision_hash: None,
        }
    }
}

/// One control-scheduled run (TASK-0091, AC2): the generation identity plus
/// the frozen config revision the run was planned under (additive — the
/// legacy shape keeps only `runId`). The revision is read under the same
/// shared-config lock as the plan, so a run concurrent with reload binds to
/// exactly one revision.
#[derive(Clone, Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ScheduledRun {
    pub run_id: u64,
    /// Immutable config revision the run was frozen under; omitted for
    /// legacy servers that never observe reload.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub revision: Option<u64>,
    /// Non-secret semantic hash of the frozen revision.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub revision_hash: Option<String>,
}

/// Typed control-run failure (TASK-0091, AC7): a stale target (not in the
/// current revision) is an actionable typed outcome, never a generic server
/// error that the agent would have to parse. Maps to one stable RPC code.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum ControlRunError {
    /// The requested target does not exist in the current revision.
    TargetNotFound { target: String },
    /// Any other scheduling failure (busy-run cancel, worker error).
    Internal(String),
}

impl ControlRunError {
    /// Maps the typed error to a stable RPC error triple.
    pub fn to_rpc(self) -> (i64, &'static str, Option<serde_json::Value>) {
        match self {
            ControlRunError::TargetNotFound { target } => (
                -32016,
                "target_not_found",
                Some(serde_json::json!({
                    "target": target,
                    "action": "reobserve-targets",
                })),
            ),
            ControlRunError::Internal(message) => {
                (-32000, "Server error", Some(serde_json::json!(message)))
            }
        }
    }
}

type RunTarget = Arc<dyn Fn(String, bool) -> Result<ScheduledRun, ControlRunError> + Send + Sync>;

/// Resolves the live target list at request time (TASK-0091, AC6): the
/// strategy reads the SHARED watch config, so `targets` after a valid reload
/// reflects the new jobs without rebuilding the server. None = static list.
pub type TargetsProvider = Arc<dyn Fn() -> Vec<ControlTarget> + Send + Sync>;
type EmitPath = Arc<dyn Fn(String) -> Result<EmitOutcome, String> + Send + Sync>;
type CancelTarget = Arc<dyn Fn(u64) -> Result<CancelResult, String> + Send + Sync>;

/// Bounded concurrent client threads; waiters never starve the accept loop.
const MAX_CLIENT_THREADS: usize = 64;

pub struct ControlServer {
    path: PathBuf,
    stop: Arc<AtomicBool>,
    thread: Option<JoinHandle<()>>,
}

#[derive(Clone)]
pub struct ControlApi {
    state: Arc<Mutex<WatcherState>>,
    targets: Vec<ControlTarget>,
    targets_provider: Option<TargetsProvider>,
    run_target: Option<RunTarget>,
    emit_path: Option<EmitPath>,
    coordinator: Option<Arc<AwaitCoordinator>>,
    outputs: Option<Arc<OutputRegistry>>,
    cancel_generation: Option<CancelTarget>,
    instance: Arc<WatcherInstance>,
    broker: Option<Arc<SnapshotBroker>>,
    estimates: Option<TargetEstimateProvider>,
    lifecycle: Option<Arc<crate::config_lifecycle::ConfigLifecycle>>,
}

impl ControlApi {
    pub fn new(state: Arc<Mutex<WatcherState>>) -> Self {
        Self {
            state,
            targets: vec![],
            targets_provider: None,
            run_target: None,
            emit_path: None,
            coordinator: None,
            outputs: None,
            cancel_generation: None,
            instance: Arc::new(WatcherInstance::new()),
            broker: None,
            estimates: None,
            lifecycle: None,
        }
    }

    pub fn with_targets(mut self, targets: Vec<ControlTarget>) -> Self {
        self.targets = targets;
        self
    }

    pub fn with_targets_provider(mut self, provider: TargetsProvider) -> Self {
        self.targets_provider = Some(provider);
        self
    }

    pub fn with_run<F>(mut self, run_target: F) -> Self
    where
        F: Fn(String, bool) -> Result<ScheduledRun, ControlRunError> + Send + Sync + 'static,
    {
        self.run_target = Some(Arc::new(run_target));
        self
    }

    pub fn with_emit<E>(mut self, emit_path: E) -> Self
    where
        E: Fn(String) -> Result<EmitOutcome, String> + Send + Sync + 'static,
    {
        self.emit_path = Some(Arc::new(emit_path));
        self
    }

    pub fn with_awaiting(mut self, coordinator: Arc<AwaitCoordinator>) -> Self {
        self.coordinator = Some(coordinator);
        self
    }

    pub fn with_outputs(mut self, outputs: Arc<OutputRegistry>) -> Self {
        self.outputs = Some(outputs);
        self
    }

    pub fn with_cancel<C>(mut self, cancel_generation: C) -> Self
    where
        C: Fn(u64) -> Result<CancelResult, String> + Send + Sync + 'static,
    {
        self.cancel_generation = Some(Arc::new(cancel_generation));
        self
    }

    pub fn with_instance(mut self, instance: Arc<WatcherInstance>) -> Self {
        self.instance = instance;
        self
    }

    pub fn with_snapshots(mut self, broker: Arc<SnapshotBroker>) -> Self {
        self.broker = Some(broker);
        self
    }

    pub fn with_estimates(mut self, estimates: TargetEstimateProvider) -> Self {
        self.estimates = Some(estimates);
        self
    }

    pub fn with_lifecycle(
        mut self,
        lifecycle: Arc<crate::config_lifecycle::ConfigLifecycle>,
    ) -> Self {
        self.lifecycle = Some(lifecycle);
        self
    }

    fn validate(&self) -> Result<(), &'static str> {
        if self.broker.is_some() && self.coordinator.is_none() {
            return Err("snapshot subscription requires await coordinator");
        }
        if self.emit_path.is_some() && self.run_target.is_none() {
            return Err("emit capability requires run capability");
        }
        if self.cancel_generation.is_some() && self.run_target.is_none() {
            return Err("cancel capability requires run capability");
        }
        if let Some(broker) = &self.broker {
            if broker.instance().token != self.instance.token {
                return Err("snapshot broker and control API must share watcher instance");
            }
        }
        Ok(())
    }
}

impl ControlServer {
    pub fn bind(path: &Path, api: ControlApi) -> io::Result<Self> {
        api.validate()
            .map_err(|message| io::Error::new(io::ErrorKind::InvalidInput, message))?;
        prepare_socket_path(path)?;
        let listener = UnixListener::bind(path)?;
        fs::set_permissions(path, fs::Permissions::from_mode(0o600))?;
        listener.set_nonblocking(true)?;

        let stop = Arc::new(AtomicBool::new(false));
        let thread_stop = Arc::clone(&stop);
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
                        let api = api.clone();
                        let clients = Arc::clone(&active_clients);
                        std::thread::spawn(move || {
                            handle_client(stream, &api);
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

fn handle_client(mut stream: UnixStream, api: &ControlApi) {
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
    let state = &api.state;
    let coordinator = api.coordinator.as_ref();
    let outputs = api.outputs.as_deref();
    let instance = api.instance.as_ref();
    let broker = api.broker.as_ref();
    let lifecycle = api.lifecycle.as_ref();

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

        // `subscribe` dedicates the connection to a notification stream:
        // it returns one immediate snapshot then streams `snapshot`
        // notifications, so it never returns to the request loop.
        if request.get("method").and_then(serde_json::Value::as_str) == Some("subscribe") {
            handle_subscribe(&mut stream, request, broker);
            return;
        }

        // `await` blocks for up to its timeout, so it needs the live stream
        // for disconnect detection and is handled outside the dispatcher; it
        // restores blocking mode before returning to the loop.
        if request.get("method").and_then(serde_json::Value::as_str) == Some("await")
            && request
                .get("params")
                .is_some_and(|params| params.is_object())
        {
            handle_await(
                &mut stream,
                request,
                state,
                coordinator,
                outputs,
                instance,
                lifecycle,
            );
            continue;
        }

        if let Some(response) = process_payload(request, api) {
            write_response(&mut stream, response);
        }
    }
}

/// Serves one `subscribe` connection (TASK-0050): returns the immediate
/// correlated snapshot, then streams `snapshot` notifications as the shared
/// broker publishes transitions. The loop ends on write failure (disconnect)
/// or broker drop (watcher shutdown), which releases the subscriber promptly.
fn handle_subscribe(
    stream: &mut UnixStream,
    request: serde_json::Value,
    broker: Option<&Arc<SnapshotBroker>>,
) {
    let id = request_id(&request);
    if request.get("jsonrpc") != Some(&serde_json::json!("2.0")) {
        write_response(stream, rpc_error(id, -32600, "Invalid Request", None));
        return;
    }
    let Some(broker) = broker else {
        write_response(
            stream,
            rpc_error(
                id,
                -32000,
                "Server error",
                Some(serde_json::json!("subscription is unavailable")),
            ),
        );
        return;
    };

    let (receiver, snapshot) = broker.subscribe();
    write_response(
        stream,
        serde_json::json!({
            "jsonrpc": "2.0",
            "id": id,
            "result": serde_json::to_value(snapshot).unwrap_or(serde_json::Value::Null),
        }),
    );

    loop {
        match receiver.recv() {
            Ok(snapshot) => {
                let notification = serde_json::json!({
                    "jsonrpc": "2.0",
                    "method": "snapshot",
                    "params": serde_json::to_value(&snapshot).unwrap_or(serde_json::Value::Null),
                });
                if writeln!(stream, "{}", notification).is_err() {
                    return;
                }
            }
            Err(_) => return,
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
    state: &Arc<Mutex<WatcherState>>,
    coordinator: Option<&Arc<AwaitCoordinator>>,
    outputs: Option<&OutputRegistry>,
    instance: &WatcherInstance,
    lifecycle: Option<&Arc<crate::config_lifecycle::ConfigLifecycle>>,
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
    let result: AwaitResult = coordinator.await_generation(
        params.mode,
        params.timeout,
        state,
        Some(&mut probe),
        outputs,
        instance.token.as_str(),
        lifecycle.map(|lifecycle| lifecycle.as_ref()),
    );
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

fn process_payload(request: serde_json::Value, api: &ControlApi) -> Option<serde_json::Value> {
    let serde_json::Value::Array(requests) = request else {
        return process_request(request, api);
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
        .filter_map(|request| process_request(request, api))
        .collect();
    if responses.is_empty() {
        return None;
    }
    Some(serde_json::Value::Array(responses))
}

fn process_request(request: serde_json::Value, api: &ControlApi) -> Option<serde_json::Value> {
    let state = &api.state;
    let targets = api.targets.as_slice();
    let targets_provider = api.targets_provider.as_ref();
    let run_target = api.run_target.as_ref();
    let emit_path = api.emit_path.as_ref();
    let outputs = api.outputs.as_deref();
    let cancel_generation = api.cancel_generation.as_ref();
    let instance = api.instance.as_ref();
    let broker = api.broker.as_ref();
    let estimates = api.estimates.as_ref();
    let lifecycle = api.lifecycle.as_ref();

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
        "status" => status_result(state, outputs, instance),
        "targets" => Ok(targets_result(
            targets_provider
                .map(|provider| provider())
                .as_deref()
                .unwrap_or(targets),
            estimates,
        )),
        "run" => run_requested_target(&request, run_target),
        "emit" => emit_requested_path(&request, emit_path),
        "cancel" => cancel_requested_generation(&request, cancel_generation, instance),
        "output" => output_retrieval(&request, outputs, instance),
        // Honest negotiated profile (contract §8): methods list only what this
        // server implements; features stay false until the additive contract
        // (subscribe, cancel, output, correlated snapshots, estimates) lands.
        // The extension keeps the legacy polling fallback and never assumes
        // capabilities from package versions.
        "config" => config_result(lifecycle),
        "capabilities" => Ok(capabilities_result(
            instance,
            broker.is_some(),
            estimates.is_some(),
            lifecycle.is_some(),
            // TASK-0073: the sequential run override is implemented whenever a
            // run target handler is wired (always in production); capability
            // negotiation lets old clients detect support before sending the
            // flag, and lets the extension fail closed on legacy servers.
            run_target.is_some(),
        )),
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

/// Negotiated capabilities (contract §6/§7): protocol facts only. `subscribe`
/// and the `subscription` feature are advertised only when a broker endpoint
/// is actually registered, so clients never assume a push stream that does
/// not exist. `durationEstimates` is advertised only when an estimate
/// provider is wired, with its declared bounds. The `config` lifecycle
/// method is advertised only when a lifecycle source is wired (TASK-0091,
/// AC3).
fn capabilities_result(
    instance: &WatcherInstance,
    subscription: bool,
    duration_estimates: bool,
    config_lifecycle: bool,
    sequential_override: bool,
) -> serde_json::Value {
    let mut methods = vec![
        "status",
        "targets",
        "run",
        "emit",
        "await",
        "output",
        "cancel",
        "capabilities",
    ];
    if subscription {
        methods.push("subscribe");
    }
    if config_lifecycle {
        methods.push("config");
    }
    let mut limits = serde_json::json!({
        "outputRetentionBytes": OUTPUT_RETENTION_BYTES as u64,
        "maxResponseBytes": MAX_RESPONSE_BYTES,
        "maxEvidenceLines": DEFAULT_FAILURE_EVIDENCE_LINES,
        // Contract §4: paging and envelope facts so advanced clients negotiate
        // before requesting, instead of discovering a > transport response.
        "outputSchemaVersion": 2,
        "outputModes": ["tail", "page"],
        "outputPageSizeMax": crate::output::OUTPUT_PAGE_MAX_BYTES as u64,
        "outputMaxBytesEffective": crate::output::DEFAULT_PAGE_BYTES as u64,
    });
    if duration_estimates {
        limits["durationEstimateLimits"] = serde_json::json!({
            "maxSamples": crate::duration_history::SUCCESS_RETENTION,
            "floorMs": crate::duration_history::DEFAULT_FLOOR_MS,
            "capMs": crate::duration_history::ABSOLUTE_CAP_MS,
        });
    }
    serde_json::json!({
        "protocolVersion": "1.0",
        "schemaVersion": 1,
        "watcherVersion": env!("CARGO_PKG_VERSION"),
        "instance": {
            "token": instance.token,
            "startedAtEpochMs": instance.started_at_epoch_ms,
        },
        "methods": methods,
        "optionalFields": [
            "batch",
            "changed",
            "predecessor",
            "supersededBy",
            "failureEvidence",
            "estimate"
        ],
        "outputFormats": ["toon", "json"],
        "limits": limits,
        "features": {
            "atomicAwait": true,
            "subscription": subscription,
            "correlatedSnapshots": false,
            "outputRetrieval": true,
            "pendingWork": false,
            "durationEstimates": duration_estimates,
            "sequentialOverride": sequential_override,
        },
    })
}

/// `config` result (TASK-0091, AC3): the live config lifecycle transition
/// plus the bounded transition history from the same shared state source.
/// Returns a typed unavailable error on servers without a lifecycle source.
fn config_result(
    lifecycle: Option<&Arc<crate::config_lifecycle::ConfigLifecycle>>,
) -> Result<serde_json::Value, (i64, &'static str, Option<serde_json::Value>)> {
    let Some(lifecycle) = lifecycle else {
        return Err((
            -32017,
            "config_lifecycle_unavailable",
            Some(serde_json::json!({ "feature": "configLifecycle" })),
        ));
    };
    let current = lifecycle.current();
    let history = lifecycle.history();
    let mut value = serde_json::to_value(&current).map_err(|_| {
        (
            -32000,
            "Server error",
            Some(serde_json::json!("config lifecycle serialization failed")),
        )
    })?;
    if let Ok(history) = serde_json::to_value(&history) {
        value["history"] = history;
    }
    Ok(value)
}

/// `targets` result: each target carries its current estimate computed at
/// request time (TASK-0055, contract §6) — never frozen at server start.
/// When no provider is wired, the legacy shape is unchanged (no estimate
/// key at all, never null).
fn targets_result(
    targets: &[ControlTarget],
    estimates: Option<&TargetEstimateProvider>,
) -> serde_json::Value {
    serde_json::json!(targets
        .iter()
        .map(|target| {
            let estimate = estimates.and_then(|provider| provider(target));
            let mut value = serde_json::to_value(target).unwrap_or_default();
            if let Some(estimate) = estimate {
                if let Ok(estimate) = serde_json::to_value(&estimate) {
                    value["estimate"] = estimate;
                }
            }
            value
        })
        .collect::<Vec<_>>())
}

/// `status` result: the legacy snapshot plus additive failure evidence when
/// the latest generation failed and retained output exists (contract §6).
fn status_result(
    state: &Arc<Mutex<WatcherState>>,
    outputs: Option<&OutputRegistry>,
    instance: &WatcherInstance,
) -> Result<serde_json::Value, (i64, &'static str, Option<serde_json::Value>)> {
    let snapshot = state.lock().unwrap().clone();
    let mut value = serde_json::to_value(snapshot.clone()).map_err(|_| {
        (
            -32000,
            "Server error",
            Some(serde_json::json!("status serialization failed")),
        )
    })?;
    if snapshot.state() == &WatcherExecutionState::Failed {
        let failed_tasks: Vec<String> = snapshot
            .tasks()
            .iter()
            .filter(|task| {
                matches!(
                    task.state,
                    crate::executor::TaskState::Failed | crate::executor::TaskState::TimedOut
                )
            })
            .map(|task| task.name.clone())
            .collect();
        if let (Some(outputs), Some(evidence)) = (
            outputs,
            outputs.and_then(|outputs| {
                outputs.failure_evidence(
                    snapshot.generation(),
                    DEFAULT_FAILURE_EVIDENCE_LINES,
                    &instance.token,
                    &failed_tasks,
                )
            }),
        ) {
            let _ = outputs;
            if let Ok(evidence) = serde_json::to_value(evidence) {
                value["failureEvidence"] = evidence;
            }
        }
    }
    Ok(value)
}

/// `output` retrieval (contract §6/§3): bounded, per generation/task/stream,
/// tail or full. Instance token is validated before registry lookup (stale
/// token cannot read a same-number generation from a replacement watcher);
/// registry failures map to typed codes `-32010`/`-32011` with structured
/// data. Generic `-32000` is reserved for genuine server failure.
fn output_retrieval(
    request: &serde_json::Value,
    outputs: Option<&OutputRegistry>,
    instance: &WatcherInstance,
) -> Result<serde_json::Value, (i64, &'static str, Option<serde_json::Value>)> {
    let Some(outputs) = outputs else {
        return Err((
            -32014,
            "output_unavailable",
            Some(serde_json::json!({ "feature": "outputRetrieval" })),
        ));
    };
    let params = request.get("params").and_then(serde_json::Value::as_object);
    let Some(generation) = params
        .and_then(|params| params.get("generation"))
        .and_then(serde_json::Value::as_u64)
    else {
        return Err((
            -32602,
            "Invalid params",
            Some(serde_json::json!(
                "output requires a numeric params.generation"
            )),
        ));
    };

    // Instance identity (contract §3 `-32012`): a request carrying a stale
    // token was formed against another watcher process and must never read
    // the same-number generation from this one. Missing token (legacy) keeps
    // working but never claims exact freshness.
    if let Some(token) = params
        .and_then(|params| params.get("instanceToken"))
        .and_then(serde_json::Value::as_str)
    {
        if token != instance.token {
            return Err((
                -32012,
                "instance_mismatch",
                Some(serde_json::json!({
                    "instance": token,
                    "activeInstance": instance.token,
                    "action": "restart-or-reobserve",
                })),
            ));
        }
    }

    let task = params
        .and_then(|params| params.get("task"))
        .and_then(serde_json::Value::as_str)
        .filter(|task| !task.trim().is_empty())
        .map(str::to_owned);
    let stream = params
        .and_then(|params| params.get("stream"))
        .and_then(serde_json::Value::as_str)
        .and_then(|stream| match stream {
            "stdout" => Some(crate::output::RetrievalStream::Stdout),
            "stderr" => Some(crate::output::RetrievalStream::Stderr),
            _ => None,
        });

    // Retrieval mode (contract §5): `mode` selects tail (last N lines per
    // stream) or page (deterministic continuation below the negotiated
    // budget). Legacy clients omit `mode` and may send `tail`/`full`;
    // unsafe unpaged `full` is translated to a first bounded page with
    // continuation, never a response at or above the transport budget.
    // Validation lives in the typed `RetrievalRequest` (TASK-0173); this
    // edge only extracts raw fields and maps typed errors to wire codes.
    let mode = params
        .and_then(|params| params.get("mode"))
        .and_then(serde_json::Value::as_str)
        .map(str::trim)
        .filter(|mode| !mode.is_empty());
    let tail = params
        .and_then(|params| params.get("tail"))
        .and_then(serde_json::Value::as_u64)
        .filter(|tail| *tail > 0)
        .map(|tail| tail as usize);
    let full = params
        .and_then(|params| params.get("full"))
        .and_then(serde_json::Value::as_bool)
        .unwrap_or(false);
    let max_bytes = params
        .and_then(|params| params.get("maxBytes"))
        .and_then(serde_json::Value::as_u64)
        .filter(|bytes| *bytes > 0)
        .map(|bytes| bytes as usize);
    let cursor = params
        .and_then(|params| params.get("cursor"))
        .and_then(serde_json::Value::as_str)
        .filter(|cursor| !cursor.trim().is_empty())
        .map(str::to_owned);

    // Contract §2: `tail` and page/full variants are structurally exclusive
    // and invalid shapes are rejected before transport — never exposed as a
    // combination the server would have to resolve ambiguously.
    let request = crate::output::RetrievalRequest::build(
        generation, task, stream, mode, tail, full, max_bytes, cursor,
    )
    .map_err(request_error_to_rpc)?;

    match request.mode {
        crate::output::RetrievalMode::Page { budget, cursor } => outputs
            .retrieve_page(
                request.generation,
                request.task.as_deref(),
                request.stream.map(crate::output::RetrievalStream::as_str),
                budget,
                cursor.as_deref(),
            )
            .map_err(|error| typed_output_error(error, request.generation))
            .and_then(serialize_retrieved),
        crate::output::RetrievalMode::Tail { lines } => outputs
            .retrieve(
                request.generation,
                request.task.as_deref(),
                request.stream.map(crate::output::RetrievalStream::as_str),
                lines,
                false,
            )
            .map_err(|error| typed_output_error(error, request.generation))
            .and_then(serialize_retrieved),
    }
}

/// Maps one typed retrieval-request validation failure to its stable wire
/// error (TASK-0173): codes and payloads are byte-identical to the previous
/// inline JSON validation.
fn request_error_to_rpc(
    error: crate::output::RequestError,
) -> (i64, &'static str, Option<serde_json::Value>) {
    match error {
        crate::output::RequestError::InvalidMode { got } => (
            -32013,
            "invalid_options",
            Some(serde_json::json!({
                "field": "mode",
                "reason": format!("output mode must be 'tail' or 'page', got '{got}'"),
                "valid": ["tail", "page"],
            })),
        ),
        crate::output::RequestError::TailCannotCarryPageOptions => (
            -32013,
            "invalid_options",
            Some(serde_json::json!({
                "field": "mode/tail",
                "reason": "tail mode cannot carry page/full or cursor options",
                "valid": ["tail", "page"],
            })),
        ),
        crate::output::RequestError::CursorRequiresPage => (
            -32013,
            "invalid_options",
            Some(serde_json::json!({
                "field": "cursor",
                "reason": "cursor requires page mode",
                "valid": ["page"],
            })),
        ),
        crate::output::RequestError::PageCannotCarryTail => (
            -32013,
            "invalid_options",
            Some(serde_json::json!({
                "field": "tail/page",
                "reason": "page mode cannot carry params.tail",
                "valid": ["page", "tail"],
            })),
        ),
    }
}

fn typed_output_error(
    error: crate::output::RetrievalError,
    generation: u64,
) -> (i64, &'static str, Option<serde_json::Value>) {
    match error {
        crate::output::RetrievalError::GenerationNotFound { retained } => (
            -32010,
            "generation_not_found",
            Some(serde_json::json!({
                "generation": generation,
                "retained": retained,
                "action": "reobserve",
            })),
        ),
        crate::output::RetrievalError::TaskNotFound {
            task,
            candidates,
            ambiguous,
        } => (
            -32011,
            "task_not_found",
            Some(serde_json::json!({
                "generation": generation,
                "task": task,
                "candidates": candidates,
                "ambiguous": ambiguous,
                "action": "reobserve-or-copy-exact",
            })),
        ),
        crate::output::RetrievalError::InvalidCursor { reason } => (
            -32013,
            "invalid_options",
            Some(serde_json::json!({
                "field": "cursor",
                "reason": reason,
                "action": "restart-paging-from-first-page",
            })),
        ),
    }
}

fn serialize_retrieved(
    retrieved: crate::output::RetrievedOutput,
) -> Result<serde_json::Value, (i64, &'static str, Option<serde_json::Value>)> {
    serde_json::to_value(&retrieved).map_err(|_| {
        (
            -32015,
            "internal",
            Some(serde_json::json!({ "kind": "serialize" })),
        )
    })
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
    // TASK-0073: optional additive `sequential` flag. Absent or false means
    // the configured concurrency applies; true requests effective concurrency
    // one for this exact generation. Malformed types are invalid params, so
    // a client can never silently get parallel execution from a typo.
    let sequential = match request
        .get("params")
        .and_then(|params| params.get("sequential"))
    {
        None => false,
        Some(serde_json::Value::Bool(value)) => *value,
        Some(_) => {
            return Err((
                -32602,
                "Invalid params",
                Some(serde_json::json!(
                    "run params.sequential must be a boolean when present"
                )),
            ))
        }
    };
    let Some(run_target) = run_target else {
        return Err((
            -32000,
            "Server error",
            Some(serde_json::json!("target execution is unavailable")),
        ));
    };

    run_target(target.to_string(), sequential)
        .map(|scheduled| serde_json::to_value(scheduled).unwrap_or_default())
        .map_err(ControlRunError::to_rpc)
}

/// `cancel` transport and identity validation (TASK-0046): a positive numeric
/// generation plus an optional instance token. The injected handler performs
/// the compare-and-act; a stale instance token is a safe no-op. Escalation is
/// reported as RPC error -32021 so clients can distinguish force cleanup.
fn cancel_requested_generation(
    request: &serde_json::Value,
    cancel_generation: Option<&CancelTarget>,
    instance: &WatcherInstance,
) -> Result<serde_json::Value, (i64, &'static str, Option<serde_json::Value>)> {
    let params = request.get("params").and_then(serde_json::Value::as_object);
    let Some(generation) = params
        .and_then(|params| params.get("generation"))
        .and_then(serde_json::Value::as_u64)
    else {
        return Err((
            -32602,
            "Invalid params",
            Some(serde_json::json!(
                "cancel requires a numeric params.generation"
            )),
        ));
    };

    // Instance identity check: a request carrying a different token was formed
    // against another watcher process and must never cancel work on this one.
    if let Some(token) = params
        .and_then(|params| params.get("instanceToken"))
        .and_then(serde_json::Value::as_str)
    {
        if token != instance.token {
            return Ok(serde_json::json!({ "cancelled": false, "generation": generation }));
        }
    }

    let Some(cancel_generation) = cancel_generation else {
        return Err((
            -32000,
            "Server error",
            Some(serde_json::json!("cancellation is unavailable")),
        ));
    };

    match cancel_generation(generation) {
        Ok(CancelResult::Cancelled {
            disposition,
            revision,
            revision_hash,
        }) => {
            // TASK-0091, AC2: the frozen config revision of the cancelled
            // generation rides the result additively.
            let mut value = serde_json::json!({ "cancelled": true, "generation": generation });
            if let Some(revision) = revision {
                value["revision"] = serde_json::json!(revision);
            }
            if let Some(revision_hash) = revision_hash {
                value["revisionHash"] = serde_json::json!(revision_hash);
            }
            match disposition {
                CancelDisposition::Graceful => Ok(value),
                CancelDisposition::Escalated => Err((
                    -32021,
                    "Cancellation escalated",
                    Some(serde_json::json!({ "escalation": true, "generation": generation })),
                )),
            }
        }
        Ok(CancelResult::Noop) => {
            Ok(serde_json::json!({ "cancelled": false, "generation": generation }))
        }
        Err(error) => Err((-32000, "Server error", Some(serde_json::json!(error)))),
    }
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
            target: None,
            execution_signature: None,
            effective_concurrency: None,
            concurrency_source: None,
            revision: Some(3),
            revision_hash: Some("abc123".to_owned()),
        }
    }

    fn instance(token: &str) -> WatcherInstance {
        WatcherInstance {
            token: token.to_owned(),
            started_at_epoch_ms: 0,
        }
    }

    #[test]
    fn control_api_rejects_snapshots_without_awaiting() {
        let state = Arc::new(Mutex::new(WatcherState::default()));
        let coordinator = Arc::new(AwaitCoordinator::new());
        let instance = Arc::new(instance("fz-test"));
        let broker = Arc::new(SnapshotBroker::new(
            instance.as_ref().clone(),
            Arc::clone(&state),
            coordinator,
        ));
        let api = ControlApi::new(state)
            .with_instance(instance)
            .with_snapshots(broker);

        assert_eq!(
            api.validate(),
            Err("snapshot subscription requires await coordinator")
        );
    }

    #[test]
    fn run_absent_sequential_defaults_to_false() {
        let seen = std::sync::Arc::new(std::sync::Mutex::new(None));
        let seen_flag = std::sync::Arc::clone(&seen);
        let run_target: RunTarget = Arc::new(move |target: String, sequential: bool| {
            *seen_flag.lock().unwrap() = Some((target, sequential));
            Ok(ScheduledRun {
                run_id: 9,
                revision: None,
                revision_hash: None,
            })
        });
        let request = serde_json::json!({ "params": { "target": "@agent-final" } });
        let result = run_requested_target(&request, Some(&run_target)).expect("run");
        assert_eq!(result, serde_json::json!({ "runId": 9 }));
        assert_eq!(
            *seen.lock().unwrap(),
            Some(("@agent-final".to_owned(), false))
        );
    }

    #[test]
    fn run_sequential_true_carries_flag() {
        let seen = std::sync::Arc::new(std::sync::Mutex::new(None));
        let seen_flag = std::sync::Arc::clone(&seen);
        let run_target: RunTarget = Arc::new(move |target: String, sequential: bool| {
            *seen_flag.lock().unwrap() = Some((target, sequential));
            Ok(ScheduledRun {
                run_id: 9,
                revision: Some(2),
                revision_hash: Some("hash-2".to_owned()),
            })
        });
        let request = serde_json::json!({
            "params": { "target": "@agent-final", "sequential": true }
        });
        let result = run_requested_target(&request, Some(&run_target)).expect("run");
        assert_eq!(
            result,
            serde_json::json!({
                "runId": 9,
                "revision": 2,
                "revisionHash": "hash-2"
            })
        );
        assert_eq!(
            *seen.lock().unwrap(),
            Some(("@agent-final".to_owned(), true))
        );
    }

    #[test]
    fn run_sequential_false_is_noop() {
        let seen = std::sync::Arc::new(std::sync::Mutex::new(None));
        let seen_flag = std::sync::Arc::clone(&seen);
        let run_target: RunTarget = Arc::new(move |target: String, sequential: bool| {
            *seen_flag.lock().unwrap() = Some((target, sequential));
            Ok(ScheduledRun {
                run_id: 9,
                revision: None,
                revision_hash: None,
            })
        });
        let request = serde_json::json!({
            "params": { "target": "@agent-final", "sequential": false }
        });
        let result = run_requested_target(&request, Some(&run_target)).expect("run");
        assert_eq!(result, serde_json::json!({ "runId": 9 }));
        assert_eq!(
            *seen.lock().unwrap(),
            Some(("@agent-final".to_owned(), false))
        );
    }

    #[test]
    fn run_sequential_malformed_type_is_invalid_params() {
        let run_target: RunTarget =
            Arc::new(|_target: String, _sequential: bool| panic!("must not run"));
        for bad in [
            serde_json::json!("yes"),
            serde_json::json!(1),
            serde_json::json!(null),
        ] {
            let request = serde_json::json!({
                "params": { "target": "@agent-final", "sequential": bad }
            });
            let (code, _, _) =
                run_requested_target(&request, Some(&run_target)).expect_err("malformed");
            assert_eq!(code, -32602);
        }
    }

    #[test]
    fn capabilities_advertise_sequential_override_only_when_supported() {
        let with = capabilities_result(&instance("fz-7f3a"), false, false, false, true);
        assert_eq!(with["features"]["sequentialOverride"], true);
        let without = capabilities_result(&instance("fz-7f3a"), false, false, false, false);
        assert_eq!(without["features"]["sequentialOverride"], false);
    }

    #[test]
    fn cancel_graceful_returns_wire_shape_matching_fixture() {
        let cancel: CancelTarget = Arc::new(|generation: u64| -> Result<CancelResult, String> {
            assert_eq!(generation, 7);
            Ok(CancelResult::Cancelled {
                disposition: CancelDisposition::Graceful,
                revision: Some(2),
                revision_hash: Some("hash-2".to_owned()),
            })
        });
        let request =
            serde_json::json!({ "params": { "generation": 7, "instanceToken": "fz-7f3a" } });
        let result = cancel_requested_generation(&request, Some(&cancel), &instance("fz-7f3a"))
            .expect("graceful cancel");
        assert_eq!(
            result,
            serde_json::json!({
                "cancelled": true,
                "generation": 7,
                "revision": 2,
                "revisionHash": "hash-2"
            })
        );
    }

    #[test]
    fn cancel_noop_returns_cancelled_false() {
        let cancel: CancelTarget = Arc::new(|_generation: u64| Ok(CancelResult::Noop));
        let request =
            serde_json::json!({ "params": { "generation": 7, "instanceToken": "fz-7f3a" } });
        let result = cancel_requested_generation(&request, Some(&cancel), &instance("fz-7f3a"))
            .expect("no-op cancel");
        assert_eq!(
            result,
            serde_json::json!({ "cancelled": false, "generation": 7 })
        );
    }

    #[test]
    fn cancel_escalated_returns_rpc_error_32021() {
        let cancel: CancelTarget = Arc::new(|_generation: u64| {
            Ok(CancelResult::Cancelled {
                disposition: CancelDisposition::Escalated,
                revision: None,
                revision_hash: None,
            })
        });
        let request =
            serde_json::json!({ "params": { "generation": 7, "instanceToken": "fz-7f3a" } });
        let (code, _, _) =
            cancel_requested_generation(&request, Some(&cancel), &instance("fz-7f3a"))
                .expect_err("escalated cancel is an RPC error");
        assert_eq!(code, -32021);
    }

    #[test]
    fn cancel_with_mismatched_instance_token_is_a_safe_noop() {
        let cancel: CancelTarget =
            Arc::new(|_generation: u64| panic!("must not be called on a stale instance"));
        let request =
            serde_json::json!({ "params": { "generation": 7, "instanceToken": "fz-stale" } });
        let result = cancel_requested_generation(&request, Some(&cancel), &instance("fz-current"))
            .expect("stale instance is a safe no-op");
        assert_eq!(
            result,
            serde_json::json!({ "cancelled": false, "generation": 7 })
        );
    }

    fn output_registry_with(records: &[(u64, &str, &[&str])]) -> OutputRegistry {
        let registry = OutputRegistry::new();
        for (generation, task, lines) in records {
            let handle = crate::cmd::CaptureHandle::new();
            for line in *lines {
                handle.append(line.as_bytes(), false);
            }
            registry.record(*generation, (*task).to_owned(), handle.finish(), None, None);
        }
        registry
    }

    #[test]
    fn output_retrieval_reports_the_frozen_config_revision_of_the_generation() {
        // TASK-0091, AC2: `output` additively exposes the frozen config
        // revision the generation ran under, so evidence is attributable to
        // the exact revision that produced it.
        let registry = OutputRegistry::new();
        let handle = crate::cmd::CaptureHandle::new();
        handle.append(b"boom\n", false);
        registry.record(
            7,
            "lint".to_owned(),
            handle.finish(),
            Some(2),
            Some("hash-2".to_owned()),
        );

        let request = serde_json::json!({ "params": { "generation": 7 } });
        let result = output_retrieval(&request, Some(&registry), &instance("fz-7f3a"))
            .expect("retrieve with revision");
        assert_eq!(result["generation"], 7);
        assert_eq!(result["revision"], 2);
        assert_eq!(result["revisionHash"], "hash-2");
    }

    #[test]
    fn config_result_reports_live_phase_revision_and_bounded_history() {
        // TASK-0091, AC3: the `config` method serves the same state source
        // the reload thread writes — phase, revision, and bounded history.
        let lifecycle = Arc::new(crate::config_lifecycle::ConfigLifecycle::new());
        lifecycle.reloaded(&crate::config_revision::ConfigRevision {
            number: 2,
            hash: "hash-2".to_owned(),
        });
        lifecycle.reloaded(&crate::config_revision::ConfigRevision {
            number: 3,
            hash: "hash-3".to_owned(),
        });

        let value = config_result(Some(&lifecycle)).expect("config");
        assert_eq!(value["phase"], "configReloaded");
        assert_eq!(value["revision"], 3);
        assert_eq!(value["revisionHash"], "hash-3");
        assert_eq!(value["ordinal"], 2);
        let history = value["history"].as_array().expect("history");
        assert_eq!(history.len(), 2);
        assert_eq!(history[0]["revision"], 2);
        assert_eq!(history[1]["revision"], 3);
    }

    #[test]
    fn config_result_without_a_lifecycle_source_is_typed_unavailable() {
        let (code, message, data) = config_result(None).expect_err("no lifecycle");
        assert_eq!(code, -32017);
        assert_eq!(message, "config_lifecycle_unavailable");
        assert_eq!(data.unwrap()["feature"], "configLifecycle");
    }

    #[test]
    fn capabilities_advertise_config_only_when_lifecycle_wired() {
        let with = capabilities_result(&instance("fz-7f3a"), false, false, true, false);
        let methods: Vec<_> = with["methods"]
            .as_array()
            .unwrap()
            .iter()
            .map(|method| method.as_str().unwrap())
            .collect();
        assert!(methods.contains(&"config"), "methods: {methods:?}");

        let without = capabilities_result(&instance("fz-7f3a"), false, false, false, false);
        let methods: Vec<_> = without["methods"]
            .as_array()
            .unwrap()
            .iter()
            .map(|method| method.as_str().unwrap())
            .collect();
        assert!(!methods.contains(&"config"), "methods: {methods:?}");
    }

    #[test]
    fn run_target_not_found_maps_to_typed_code() {
        let run_target: RunTarget = Arc::new(|_target: String, _sequential: bool| {
            Err(ControlRunError::TargetNotFound {
                target: "gone".to_owned(),
            })
        });
        let request = serde_json::json!({ "params": { "target": "gone" } });
        let (code, message, data) =
            run_requested_target(&request, Some(&run_target)).expect_err("stale target");
        assert_eq!(code, -32016);
        assert_eq!(message, "target_not_found");
        let data = data.expect("structured data");
        assert_eq!(data["target"], "gone");
        assert_eq!(data["action"], "reobserve-targets");
    }

    #[test]
    fn output_mismatched_instance_token_is_typed_instance_error() {
        let outputs = output_registry_with(&[(7, "t", &["x\n"])]);
        let request = serde_json::json!({
            "params": { "generation": 7, "instanceToken": "fz-stale" }
        });
        let (code, message, data) =
            output_retrieval(&request, Some(&outputs), &instance("fz-current"))
                .expect_err("stale instance must fail");
        assert_eq!(code, -32012);
        assert_eq!(message, "instance_mismatch");
        let data = data.expect("structured data");
        assert_eq!(data["activeInstance"], "fz-current");
        assert_eq!(data["action"], "restart-or-reobserve");
    }

    #[test]
    fn output_matching_instance_token_reads_registry() {
        let outputs = output_registry_with(&[(7, "t", &["x\n"])]);
        let request = serde_json::json!({
            "params": { "generation": 7, "instanceToken": "fz-7f3a" }
        });
        let result = output_retrieval(&request, Some(&outputs), &instance("fz-7f3a"))
            .expect("matching instance reads");
        assert_eq!(result["generation"], 7);
    }

    #[test]
    fn output_missing_generation_maps_to_typed_code() {
        let outputs = output_registry_with(&[(5, "t", &["x\n"])]);
        let request = serde_json::json!({ "params": { "generation": 9 } });
        let (code, message, data) =
            output_retrieval(&request, Some(&outputs), &instance("fz-7f3a"))
                .expect_err("missing generation");
        assert_eq!(code, -32010);
        assert_eq!(message, "generation_not_found");
        let data = data.expect("structured data");
        assert_eq!(data["retained"], serde_json::json!([5]));
        assert_eq!(data["action"], "reobserve");
    }

    #[test]
    fn output_missing_task_maps_to_typed_code_with_candidates() {
        let outputs = output_registry_with(&[(7, "lint @fast", &["x\n"])]);
        let request = serde_json::json!({ "params": { "generation": 7, "task": "nope" } });
        let (code, message, data) =
            output_retrieval(&request, Some(&outputs), &instance("fz-7f3a"))
                .expect_err("unknown task");
        assert_eq!(code, -32011);
        assert_eq!(message, "task_not_found");
        let data = data.expect("structured data");
        assert_eq!(data["task"], "nope");
        assert_eq!(data["candidates"], serde_json::json!(["lint @fast"]));
        assert_eq!(data["ambiguous"], false);
        assert_eq!(data["action"], "reobserve-or-copy-exact");
    }

    #[test]
    fn output_tail_and_full_together_is_typed_invalid_options() {
        let outputs = output_registry_with(&[(7, "t", &["x\n"])]);
        let request = serde_json::json!({
            "params": { "generation": 7, "tail": 40, "full": true }
        });
        let (code, message, _) = output_retrieval(&request, Some(&outputs), &instance("fz-7f3a"))
            .expect_err("tail+full conflict");
        assert_eq!(code, -32013);
        assert_eq!(message, "invalid_options");
    }

    #[test]
    fn output_unavailable_registry_is_typed_unavailable() {
        let request = serde_json::json!({ "params": { "generation": 7 } });
        let (code, message, data) =
            output_retrieval(&request, None, &instance("fz-7f3a")).expect_err("registry not wired");
        assert_eq!(code, -32014);
        assert_eq!(message, "output_unavailable");
        assert_eq!(data.unwrap()["feature"], "outputRetrieval");
    }

    #[test]
    fn output_canonical_task_resolution_reports_selected_exact_id() {
        let outputs = output_registry_with(&[(7, "run integration @agent-final", &["boom\n"])]);
        let request =
            serde_json::json!({ "params": { "generation": 7, "task": "run integration" } });
        let result = output_retrieval(&request, Some(&outputs), &instance("fz-7f3a"))
            .expect("single canonical candidate resolves");
        assert_eq!(result["resolvedTask"], "run integration @agent-final");
        assert_eq!(result["tasks"][0]["id"], "run integration @agent-final");
    }

    #[test]
    fn output_page_mode_returns_bounded_first_page_with_continuation() {
        let outputs = output_registry_with(&[(7, "t", &["aaa\n", "bbb\n", "ccc\n"])]);
        // Budget derived from a page containing exactly the first two lines,
        // so the first page stops with a continuation and the second finishes.
        let two_lines = crate::output::RetrievedOutput {
            generation: 7,
            revision: None,
            revision_hash: None,
            resolved_task: None,
            tasks: vec![crate::output::RetrievedTask {
                id: "t".to_owned(),
                stdout: Some(crate::output::StreamOutput {
                    content: "aaa\nbbb\n".to_owned(),
                    lines: 2,
                    retained_bytes: 12,
                    observed_bytes: 12,
                    truncated: false,
                }),
                stderr: None,
            }],
            next_cursor: Some("cursor".to_owned()),
            returned_bytes: Some(8),
            truncated: Some(true),
        };
        let budget = serde_json::to_vec(&two_lines)
            .expect("serialize reference")
            .len();
        let request = serde_json::json!({
            "params": { "generation": 7, "mode": "page", "maxBytes": budget }
        });
        let result =
            output_retrieval(&request, Some(&outputs), &instance("fz-7f3a")).expect("page mode");
        assert_eq!(result["generation"], 7);
        assert!(result["returnedBytes"].as_u64().unwrap() > 0);
        assert!(result["truncated"].as_bool().unwrap());
        let cursor = result["nextCursor"].as_str().expect("cursor");
        assert!(
            cursor.starts_with("7|"),
            "generation-scoped cursor: {cursor}"
        );

        // Follow the cursor: the second page continues exactly, then ends.
        let next = serde_json::json!({
            "params": { "generation": 7, "mode": "page", "maxBytes": budget, "cursor": cursor }
        });
        let second =
            output_retrieval(&next, Some(&outputs), &instance("fz-7f3a")).expect("second page");
        assert!(!second["truncated"].as_bool().unwrap());
        assert!(second["nextCursor"].is_null() || second.get("nextCursor").is_none());
    }

    #[test]
    fn output_page_mode_rejects_tail_and_unknown_modes() {
        let outputs = output_registry_with(&[(7, "t", &["x\n"])]);
        // page + tail is structurally exclusive (contract §2).
        let page_with_tail = serde_json::json!({
            "params": { "generation": 7, "mode": "page", "tail": 40 }
        });
        let (code, _, _) = output_retrieval(&page_with_tail, Some(&outputs), &instance("fz-7f3a"))
            .expect_err("page+tail conflict");
        assert_eq!(code, -32013);

        // Unknown mode is invalid options.
        let bad_mode = serde_json::json!({ "params": { "generation": 7, "mode": "dump" } });
        let (code, _, data) = output_retrieval(&bad_mode, Some(&outputs), &instance("fz-7f3a"))
            .expect_err("unknown mode");
        assert_eq!(code, -32013);
        assert_eq!(data.unwrap()["valid"], serde_json::json!(["tail", "page"]));
    }

    #[test]
    fn output_cursor_without_page_mode_is_invalid_options() {
        let outputs = output_registry_with(&[(7, "t", &["x\n"])]);
        let request = serde_json::json!({ "params": { "generation": 7, "cursor": "7|0|0|0" } });
        let (code, _, _) = output_retrieval(&request, Some(&outputs), &instance("fz-7f3a"))
            .expect_err("cursor requires page");
        assert_eq!(code, -32013);
    }

    #[test]
    fn output_tampered_cursor_maps_to_typed_invalid_options() {
        let outputs = output_registry_with(&[(7, "t", &["x\n"])]);
        let request = serde_json::json!({
            "params": { "generation": 7, "mode": "page", "cursor": "7|9|0|0" }
        });
        let (code, message, data) =
            output_retrieval(&request, Some(&outputs), &instance("fz-7f3a"))
                .expect_err("tampered cursor");
        assert_eq!(code, -32013);
        assert_eq!(message, "invalid_options");
        assert_eq!(data.unwrap()["field"], "cursor");
    }

    #[test]
    fn output_legacy_full_translates_to_bounded_page_with_continuation() {
        let outputs = output_registry_with(&[(7, "t", &["aaa\n", "bbb\n", "ccc\n"])]);
        // Legacy `full: true` must never return a response at or above the
        // transport budget: it becomes a first bounded page.
        let request = serde_json::json!({ "params": { "generation": 7, "full": true } });
        let result = output_retrieval(&request, Some(&outputs), &instance("fz-7f3a"))
            .expect("legacy full translates to a page");
        let serialized = serde_json::to_vec(&result).expect("serialize");
        assert!(
            serialized.len() <= crate::output::DEFAULT_PAGE_BYTES,
            "legacy full must stay under page budget, got {}",
            serialized.len()
        );
    }

    #[test]
    fn capabilities_advertise_output_paging_model() {
        let caps = capabilities_result(&instance("fz-7f3a"), false, false, false, false);
        let limits = &caps["limits"];
        assert_eq!(limits["outputSchemaVersion"], 2);
        assert_eq!(limits["outputModes"], serde_json::json!(["tail", "page"]));
        assert_eq!(
            limits["outputPageSizeMax"],
            serde_json::json!(crate::output::OUTPUT_PAGE_MAX_BYTES as u64)
        );
        assert!(limits["outputMaxBytesEffective"].as_u64().unwrap() > 0);
        assert!(
            limits["outputMaxBytesEffective"].as_u64().unwrap()
                < limits["maxResponseBytes"].as_u64().unwrap()
        );
    }

    #[test]
    fn status_failure_evidence_carries_structured_output_ref() {
        // Contract §1/§5: terminal status on a failed generation emits an
        // exact outputRef (instance token + generation + exact task ID + safe
        // defaults), not a human string the agent would have to parse.
        let state = Arc::new(Mutex::new(WatcherState::default()));
        state.lock().unwrap().apply(started(7, None, None));
        state.lock().unwrap().apply(Event::Finished {
            run_id: 7,
            superseded_by: None,
            elapsed: Duration::from_millis(5),
            failures: vec!["boom".to_owned()],
        });
        let registry = output_registry_with(&[(7, "lint @fast", &["error: boom\n"])]);

        let value = status_result(&state, Some(&registry), &instance("fz-7f3a")).expect("status");
        let evidence = &value["failureEvidence"];
        assert_eq!(
            evidence["retrieve"],
            "fzz control output --generation 7 --task 'lint @fast' --tail 80"
        );
        let output_ref = &evidence["outputRef"];
        assert_eq!(output_ref["instanceToken"], "fz-7f3a");
        assert_eq!(output_ref["generation"], 7);
        assert_eq!(output_ref["task"], "lint @fast");
        assert_eq!(output_ref["mode"], "tail");
        assert!(output_ref["retrieve"]
            .as_str()
            .unwrap()
            .contains("--instance 'fz-7f3a'"));
        assert_eq!(evidence["additionalFailedTasks"], 0);
    }

    #[test]
    fn status_evidence_counts_multiple_failed_tasks_via_primary_ref() {
        let state = Arc::new(Mutex::new(WatcherState::default()));
        state.lock().unwrap().apply(started(7, None, None));
        state.lock().unwrap().apply(Event::Finished {
            run_id: 7,
            superseded_by: None,
            elapsed: Duration::from_millis(5),
            failures: vec!["a".to_owned(), "b".to_owned()],
        });
        let registry =
            output_registry_with(&[(7, "lint", &["a boom\n"]), (7, "test", &["b boom\n"])]);

        let value = status_result(&state, Some(&registry), &instance("fz-7f3a")).expect("status");
        let evidence = &value["failureEvidence"];
        assert_eq!(evidence["outputRef"]["task"], "lint");
        assert_eq!(evidence["additionalFailedTasks"], 1);
    }

    #[test]
    fn capabilities_advertise_subscribe_only_when_broker_registered() {
        let with = capabilities_result(&instance("fz-7f3a"), true, false, false, false);
        let methods: Vec<_> = with["methods"]
            .as_array()
            .unwrap()
            .iter()
            .map(|method| method.as_str().unwrap())
            .collect();
        assert!(methods.contains(&"subscribe"), "methods: {methods:?}");
        assert_eq!(with["features"]["subscription"], true);

        let without = capabilities_result(&instance("fz-7f3a"), false, false, false, false);
        let methods: Vec<_> = without["methods"]
            .as_array()
            .unwrap()
            .iter()
            .map(|method| method.as_str().unwrap())
            .collect();
        assert!(!methods.contains(&"subscribe"), "methods: {methods:?}");
        assert_eq!(without["features"]["subscription"], false);
    }

    #[test]
    fn capabilities_advertise_duration_estimates_only_when_provider_wired() {
        let with = capabilities_result(&instance("fz-7f3a"), true, true, false, false);
        assert_eq!(with["features"]["durationEstimates"], true);
        assert_eq!(
            with["optionalFields"][0], "batch",
            "estimate stays an optional additive field"
        );
        assert!(with["optionalFields"]
            .as_array()
            .unwrap()
            .iter()
            .any(|field| field.as_str() == Some("estimate")));
        assert_eq!(with["limits"]["durationEstimateLimits"]["maxSamples"], 20);
        assert_eq!(with["limits"]["durationEstimateLimits"]["capMs"], 900_000);

        let without = capabilities_result(&instance("fz-7f3a"), false, false, false, false);
        assert_eq!(without["features"]["durationEstimates"], false);
        assert!(
            without["limits"].get("durationEstimateLimits").is_none(),
            "no limits declared when the surface is inactive"
        );
    }

    #[test]
    fn targets_result_omits_estimate_without_provider() {
        let targets = vec![ControlTarget {
            name: "build".to_owned(),
            commands: vec!["make build".to_owned()],
        }];
        let json = targets_result(&targets, None);
        assert_eq!(json[0]["name"], "build");
        assert_eq!(json[0]["commands"][0], "make build");
        assert!(json[0].get("estimate").is_none(), "legacy shape unchanged");
    }

    #[test]
    fn targets_result_attaches_estimate_at_request_time() {
        let targets = vec![ControlTarget {
            name: "build".to_owned(),
            commands: vec!["make build".to_owned()],
        }];
        let estimate = RunEstimate {
            typical_ms: 38_000,
            upper_ms: 61_000,
            recommended_timeout_ms: 95_000,
            samples: 12,
            confidence: crate::duration_history::EstimateConfidence::Medium,
            source: crate::duration_history::EstimateSource::Measured,
        };
        let provider: TargetEstimateProvider = Arc::new(move |target: &ControlTarget| {
            if target.name == "build" {
                Some(estimate.clone())
            } else {
                None
            }
        });
        let json = targets_result(&targets, Some(&provider));
        let estimate_json = &json[0]["estimate"];
        assert_eq!(estimate_json["typicalMs"], 38_000);
        assert_eq!(estimate_json["upperMs"], 61_000);
        assert_eq!(estimate_json["recommendedTimeoutMs"], 95_000);
        assert_eq!(estimate_json["samples"], 12);
        assert_eq!(estimate_json["confidence"], "medium");
        assert_eq!(estimate_json["source"], "measured");
        assert_eq!(json[0]["name"], "build", "legacy fields preserved");
        assert_eq!(json[0]["commands"][0], "make build");
    }

    #[test]
    fn targets_result_never_exposes_signature_or_state_path() {
        let targets = vec![ControlTarget {
            name: "build".to_owned(),
            commands: ["make build".to_owned()].to_vec(),
        }];
        let provider: TargetEstimateProvider = Arc::new(|_| {
            Some(RunEstimate {
                typical_ms: 1_000,
                upper_ms: 2_000,
                recommended_timeout_ms: 3_000,
                samples: 1,
                confidence: crate::duration_history::EstimateConfidence::Low,
                source: crate::duration_history::EstimateSource::Measured,
            })
        });
        let json = serde_json::to_string(&targets_result(&targets, Some(&provider))).unwrap();
        assert!(
            !json.contains("signature"),
            "no signature inputs on the wire"
        );
        assert!(
            !json.contains("run-durations-v1.json"),
            "no state-file path on the wire"
        );
        assert!(!json.contains("execution_signature"));
    }

    #[test]
    fn estimate_serializes_camelcase_and_skips_nothing_when_present() {
        let estimate = RunEstimate {
            typical_ms: 38_000,
            upper_ms: 61_000,
            recommended_timeout_ms: 95_000,
            samples: 12,
            confidence: crate::duration_history::EstimateConfidence::Medium,
            source: crate::duration_history::EstimateSource::Measured,
        };
        let json = serde_json::to_value(&estimate).unwrap();
        assert_eq!(json["typicalMs"], 38_000);
        assert_eq!(json["confidence"], "medium");
        assert_eq!(json["source"], "measured");
    }

    #[test]
    fn cancel_requires_a_numeric_generation() {
        let cancel: CancelTarget = Arc::new(|_generation: u64| Ok(CancelResult::Noop));
        let request = serde_json::json!({ "params": {} });
        let (code, _, _) =
            cancel_requested_generation(&request, Some(&cancel), &instance("fz-7f3a"))
                .expect_err("missing generation is invalid");
        assert_eq!(code, -32602);
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
        let state = Arc::new(Mutex::new(WatcherState::default()));
        let api = ControlApi::new(Arc::clone(&state));
        let _server = ControlServer::bind(&path, api).unwrap();

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
