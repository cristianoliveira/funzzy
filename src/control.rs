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
        Self::start_internal(path, state, None)
    }

    pub fn start_with_runner<F>(
        path: &Path,
        state: Arc<Mutex<ControlState>>,
        run_target: F,
    ) -> io::Result<Self>
    where
        F: Fn(String) -> Result<u64, String> + Send + Sync + 'static,
    {
        Self::start_internal(path, state, Some(Arc::new(run_target)))
    }

    fn start_internal(
        path: &Path,
        state: Arc<Mutex<ControlState>>,
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
                    Ok((stream, _)) => handle_client(stream, &state, run_target.as_ref()),
                    Err(err) if err.kind() == io::ErrorKind::WouldBlock => {
                        std::thread::sleep(Duration::from_millis(20));
                    }
                    Err(_) => break,
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
    run_target: Option<&RunTarget>,
) {
    let mut request = String::new();
    let read_result = BufReader::new(&stream).read_line(&mut request);
    if read_result.is_err() {
        return;
    }

    let request: serde_json::Value = match serde_json::from_str(&request) {
        Ok(request) => request,
        Err(err) => {
            write_response(
                &mut stream,
                serde_json::json!({"v": 1, "error": format!("invalid request: {}", err)}),
            );
            return;
        }
    };

    let id = request
        .get("id")
        .cloned()
        .unwrap_or(serde_json::Value::Null);
    if request.get("v") != Some(&serde_json::json!(1)) {
        write_response(
            &mut stream,
            serde_json::json!({"v": 1, "id": id, "error": "unsupported protocol version"}),
        );
        return;
    }

    match request.get("method").and_then(|method| method.as_str()) {
        Some("status") => {
            let snapshot = state.lock().unwrap().clone();
            write_response(
                &mut stream,
                serde_json::json!({"v": 1, "id": id, "result": snapshot}),
            );
        }
        Some("run") => run_requested_target(&mut stream, id, &request, run_target),
        _ => write_response(
            &mut stream,
            serde_json::json!({"v": 1, "id": id, "error": "unsupported request"}),
        ),
    }
}

fn run_requested_target(
    stream: &mut UnixStream,
    id: serde_json::Value,
    request: &serde_json::Value,
    run_target: Option<&RunTarget>,
) {
    let Some(run_target) = run_target else {
        write_response(
            stream,
            serde_json::json!({"v": 1, "id": id, "error": "target execution is unavailable"}),
        );
        return;
    };
    let Some(target) = request
        .get("params")
        .and_then(|params| params.get("target"))
        .and_then(|target| target.as_str())
        .filter(|target| !target.trim().is_empty())
    else {
        write_response(
            stream,
            serde_json::json!({"v": 1, "id": id, "error": "run requires a target"}),
        );
        return;
    };

    match run_target(target.to_string()) {
        Ok(run_id) => write_response(
            stream,
            serde_json::json!({"v": 1, "id": id, "result": {"runId": run_id}}),
        ),
        Err(error) => write_response(
            stream,
            serde_json::json!({"v": 1, "id": id, "error": error}),
        ),
    }
}

fn write_response(stream: &mut UnixStream, response: serde_json::Value) {
    let _ = writeln!(stream, "{}", response);
}
