use crate::stdout;
use crate::workers::WorkerEvent;
use serde_derive::Serialize;
use std::fs;
use std::io::{self, BufRead, BufReader, Write};
use std::os::unix::fs::PermissionsExt;
use std::os::unix::net::{UnixListener, UnixStream};
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};
use std::thread::JoinHandle;
use std::time::Duration;

#[derive(Clone, Debug, Serialize)]
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

#[derive(Clone, Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ControlState {
    generation: u64,
    state: ExecutionState,
    trigger: Option<String>,
    commands: Vec<String>,
    duration_ms: Option<u64>,
    failures: Vec<String>,
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
        }
    }
}

impl ControlState {
    pub fn apply(&mut self, event: WorkerEvent) {
        match event {
            WorkerEvent::Started {
                run_id,
                trigger,
                commands,
            } => {
                self.generation = run_id;
                self.state = ExecutionState::Running;
                self.trigger = Some(trigger);
                self.commands = commands;
                self.duration_ms = None;
                self.failures.clear();
            }
            WorkerEvent::Finished { elapsed, failures } => {
                self.state = if failures.is_empty() {
                    ExecutionState::Passed
                } else {
                    ExecutionState::Failed
                };
                self.duration_ms = Some(elapsed.as_millis() as u64);
                self.failures = failures;
            }
            WorkerEvent::Cancelled => {
                self.state = ExecutionState::Cancelled;
                self.duration_ms = None;
            }
            WorkerEvent::Tick => {}
        }
    }
}

type RunTarget = Arc<dyn Fn(String) -> Result<u64, String> + Send + Sync>;

pub struct ControlServer {
    path: PathBuf,
    stop: Arc<AtomicBool>,
    thread: Option<JoinHandle<()>>,
}

impl ControlServer {
    #[allow(dead_code)]
    pub fn start(path: &Path, state: Arc<Mutex<ControlState>>) -> io::Result<Self> {
        Self::start_internal(path, state, vec![], None)
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
        Self::start_internal(path, state, targets, Some(Arc::new(run_target)))
    }

    fn start_internal(
        path: &Path,
        state: Arc<Mutex<ControlState>>,
        targets: Vec<ControlTarget>,
        run_target: Option<RunTarget>,
    ) -> io::Result<Self> {
        prepare_socket_path(path)?;
        let listener = UnixListener::bind(path)?;
        fs::set_permissions(path, fs::Permissions::from_mode(0o600))?;
        listener.set_nonblocking(true)?;

        let stop = Arc::new(AtomicBool::new(false));
        let thread_stop = Arc::clone(&stop);
        let thread = std::thread::spawn(move || {
            while !thread_stop.load(Ordering::Relaxed) {
                match listener.accept() {
                    Ok((stream, _)) => handle_client(stream, &state, &targets, run_target.as_ref()),
                    Err(err) if err.kind() == io::ErrorKind::WouldBlock => {
                        std::thread::sleep(Duration::from_millis(20));
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
) {
    let mut request = String::new();
    if BufReader::new(&stream).read_line(&mut request).is_err() {
        return;
    }

    let request = match serde_json::from_str(&request) {
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

    if let Some(response) = process_payload(request, state, targets, run_target) {
        write_response(&mut stream, response);
    }
}

fn process_payload(
    request: serde_json::Value,
    state: &Arc<Mutex<ControlState>>,
    targets: &[ControlTarget],
    run_target: Option<&RunTarget>,
) -> Option<serde_json::Value> {
    let serde_json::Value::Array(requests) = request else {
        return process_request(request, state, targets, run_target);
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
        .filter_map(|request| process_request(request, state, targets, run_target))
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
