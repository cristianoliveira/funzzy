#[cfg(unix)]
mod unix {
    use funzzy::control::{ControlServer, ControlState};
    use funzzy::workers::WorkerEvent;
    use serde_json::Value;
    use std::io::{BufRead, BufReader, Write};
    use std::os::unix::net::UnixStream;
    use std::sync::{Arc, Mutex};
    use std::time::Duration;

    fn socket_path(name: &str) -> std::path::PathBuf {
        std::env::temp_dir().join(format!(
            "funzzy-control-{}-{}.sock",
            std::process::id(),
            name
        ))
    }

    fn status(path: &std::path::Path) -> Value {
        let mut stream = UnixStream::connect(path).expect("control socket should accept clients");
        stream
            .write_all(b"{\"v\":1,\"id\":\"test\",\"method\":\"status\"}\n")
            .unwrap();
        let mut response = String::new();
        BufReader::new(stream).read_line(&mut response).unwrap();
        serde_json::from_str(&response).unwrap()
    }

    #[test]
    fn it_reports_the_latest_execution_status() {
        let path = socket_path("status");
        let state = Arc::new(Mutex::new(ControlState::default()));
        let _server = ControlServer::start(&path, Arc::clone(&state)).unwrap();

        state.lock().unwrap().apply(WorkerEvent::Started {
            run_id: 1,
            trigger: "src/main.rs".to_string(),
            commands: vec!["cargo test".to_string()],
        });
        state.lock().unwrap().apply(WorkerEvent::Finished {
            elapsed: Duration::from_millis(42),
            failures: vec![],
        });

        let response = status(&path);
        assert_eq!(response["v"], 1);
        assert_eq!(response["id"], "test");
        assert_eq!(response["result"]["generation"], 1);
        assert_eq!(response["result"]["state"], "passed");
        assert_eq!(response["result"]["trigger"], "src/main.rs");
        assert_eq!(response["result"]["durationMs"], 42);
    }

    #[test]
    fn it_reports_failures_without_requiring_the_full_log() {
        let path = socket_path("failure");
        let state = Arc::new(Mutex::new(ControlState::default()));
        let _server = ControlServer::start(&path, Arc::clone(&state)).unwrap();

        state.lock().unwrap().apply(WorkerEvent::Started {
            run_id: 1,
            trigger: "tests/auth.rs".to_string(),
            commands: vec!["cargo test auth".to_string()],
        });
        state.lock().unwrap().apply(WorkerEvent::Finished {
            elapsed: Duration::from_secs(1),
            failures: vec!["cargo test auth exited with status 1".to_string()],
        });

        let response = status(&path);
        assert_eq!(response["result"]["state"], "failed");
        assert_eq!(response["result"]["failures"].as_array().unwrap().len(), 1);
    }

    #[test]
    fn it_schedules_a_named_target() {
        let path = socket_path("run");
        let state = Arc::new(Mutex::new(ControlState::default()));
        let requested = Arc::new(Mutex::new(Vec::new()));
        let captured = Arc::clone(&requested);
        let _server = ControlServer::start_with_runner(&path, state, move |target| {
            captured.lock().unwrap().push(target);
            Ok(7)
        })
        .unwrap();

        let mut stream = UnixStream::connect(&path).unwrap();
        stream
            .write_all(b"{\"v\":1,\"id\":\"run\",\"method\":\"run\",\"params\":{\"target\":\"@agent-final\"}}\n")
            .unwrap();
        let mut response = String::new();
        BufReader::new(stream).read_line(&mut response).unwrap();
        let response: Value = serde_json::from_str(&response).unwrap();

        assert_eq!(response["result"]["runId"], 7);
        assert_eq!(*requested.lock().unwrap(), vec!["@agent-final"]);
    }

    #[test]
    fn it_removes_the_socket_when_the_server_stops() {
        let path = socket_path("cleanup");
        let state = Arc::new(Mutex::new(ControlState::default()));
        let server = ControlServer::start(&path, state).unwrap();
        assert!(path.exists());

        drop(server);

        assert!(!path.exists());
    }
}
