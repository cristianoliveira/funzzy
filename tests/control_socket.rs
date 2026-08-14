#[cfg(unix)]
mod unix {
    use funzzy::control::{ControlServer, ControlState, ControlTarget};
    use funzzy::executor::Event as WorkerEvent;
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

    fn call(path: &std::path::Path, request: Value) -> Value {
        raw_call(path, &request.to_string())
    }

    fn raw_call(path: &std::path::Path, request: &str) -> Value {
        let mut stream = UnixStream::connect(path).expect("control socket should accept clients");
        writeln!(stream, "{}", request).unwrap();
        let mut response = String::new();
        BufReader::new(stream).read_line(&mut response).unwrap();
        serde_json::from_str(&response).unwrap()
    }

    fn status(path: &std::path::Path) -> Value {
        call(
            path,
            serde_json::json!({"jsonrpc": "2.0", "id": "test", "method": "status"}),
        )
    }

    #[test]
    fn it_reports_the_latest_execution_status() {
        let path = socket_path("status");
        let state = Arc::new(Mutex::new(ControlState::default()));
        let _server = ControlServer::start(&path, Arc::clone(&state)).unwrap();

        state.lock().unwrap().apply(WorkerEvent::Started {
            run_id: 1,
            trigger: "src/main.rs".to_string(),
            batch: None,
            predecessor: None,
            changed: vec![],
            commands: vec!["cargo test".to_string()],
        });
        state.lock().unwrap().apply(WorkerEvent::Finished {
            run_id: 1,
            superseded_by: None,
            elapsed: Duration::from_millis(42),
            failures: vec![],
        });

        let response = status(&path);
        assert_eq!(response["jsonrpc"], "2.0");
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
            batch: None,
            predecessor: None,
            changed: vec![],
            commands: vec!["cargo test auth".to_string()],
        });
        state.lock().unwrap().apply(WorkerEvent::Finished {
            run_id: 1,
            superseded_by: None,
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
        let _server = ControlServer::start_with_runner(&path, state, vec![], move |target| {
            captured.lock().unwrap().push(target);
            Ok(7)
        })
        .unwrap();

        let response = call(
            &path,
            serde_json::json!({
                "jsonrpc": "2.0",
                "id": "run",
                "method": "run",
                "params": {"target": "@agent-final"}
            }),
        );

        assert_eq!(response["jsonrpc"], "2.0");
        assert_eq!(response["result"]["runId"], 7);
        assert_eq!(*requested.lock().unwrap(), vec!["@agent-final"]);
    }

    #[test]
    fn it_lists_available_targets() {
        let path = socket_path("targets");
        let state = Arc::new(Mutex::new(ControlState::default()));
        let targets = vec![ControlTarget {
            name: "final checks @agent-final".to_owned(),
            commands: vec!["cargo test".to_owned()],
        }];
        let _server = ControlServer::start_with_runner(&path, state, targets, |_| Ok(1)).unwrap();

        let response = call(
            &path,
            serde_json::json!({"jsonrpc": "2.0", "id": "targets", "method": "targets"}),
        );

        assert_eq!(response["jsonrpc"], "2.0");
        assert_eq!(response["result"][0]["name"], "final checks @agent-final");
        assert_eq!(response["result"][0]["commands"][0], "cargo test");
    }

    #[test]
    fn it_answers_capabilities_with_the_negotiated_profile() {
        let path = socket_path("capabilities");
        let state = Arc::new(Mutex::new(ControlState::default()));
        let _server = ControlServer::start(&path, state).unwrap();

        let response = call(
            &path,
            serde_json::json!({"jsonrpc": "2.0", "id": "capabilities", "method": "capabilities"}),
        );

        assert_eq!(response["jsonrpc"], "2.0");
        let result = &response["result"];
        assert_eq!(result["protocolVersion"], "1.0");
        assert_eq!(result["schemaVersion"], 1);
        let token = result["instance"]["token"]
            .as_str()
            .expect("instance token");
        assert!(!token.is_empty());
        assert!(result["instance"]["startedAtEpochMs"].is_number());
        let methods: Vec<_> = result["methods"]
            .as_array()
            .unwrap()
            .iter()
            .map(|method| method.as_str().unwrap())
            .collect();
        for method in ["status", "targets", "run", "emit", "await", "capabilities"] {
            assert!(
                methods.contains(&method),
                "missing method {method}: {methods:?}"
            );
        }
        // Honest current profile: `atomicAwait` landed with TASK-0044; the
        // remaining additive features stay false so the extension keeps the
        // legacy polling fallback for them (contract §6).
        assert_eq!(result["features"]["atomicAwait"], true);
        assert_eq!(result["features"]["subscription"], false);
        assert_eq!(result["features"]["correlatedSnapshots"], false);
        assert_eq!(result["features"]["outputRetrieval"], false);
        assert_eq!(result["features"]["pendingWork"], false);
        assert_eq!(result["limits"]["maxResponseBytes"], 65536);
        assert_eq!(result["limits"]["maxEvidenceLines"], 40);
    }

    #[test]
    fn it_keeps_a_stable_instance_identity_across_requests() {
        let path = socket_path("capabilities-stable");
        let state = Arc::new(Mutex::new(ControlState::default()));
        let _server = ControlServer::start(&path, state).unwrap();

        let first = call(
            &path,
            serde_json::json!({"jsonrpc": "2.0", "id": "c1", "method": "capabilities"}),
        );
        let second = call(
            &path,
            serde_json::json!({"jsonrpc": "2.0", "id": "c2", "method": "capabilities"}),
        );

        assert_eq!(
            first["result"]["instance"]["token"],
            second["result"]["instance"]["token"]
        );
        assert_eq!(
            first["result"]["instance"]["startedAtEpochMs"],
            second["result"]["instance"]["startedAtEpochMs"]
        );
    }

    #[test]
    fn it_generates_a_fresh_instance_identity_per_server() {
        let first_path = socket_path("capabilities-restart-a");
        let first =
            ControlServer::start(&first_path, Arc::new(Mutex::new(ControlState::default())))
                .unwrap();
        let first_token = call(
            &first_path,
            serde_json::json!({"jsonrpc": "2.0", "id": "c", "method": "capabilities"}),
        )["result"]["instance"]["token"]
            .clone();
        drop(first);

        // A restart is a new instance: same socket path, fresh identity.
        let second_path = socket_path("capabilities-restart-b");
        let _second =
            ControlServer::start(&second_path, Arc::new(Mutex::new(ControlState::default())))
                .unwrap();
        let second_token = call(
            &second_path,
            serde_json::json!({"jsonrpc": "2.0", "id": "c", "method": "capabilities"}),
        )["result"]["instance"]["token"]
            .clone();

        assert_ne!(first_token, second_token);
    }

    #[test]
    fn it_returns_standard_json_rpc_errors() {
        let path = socket_path("errors");
        let state = Arc::new(Mutex::new(ControlState::default()));
        let _server = ControlServer::start(&path, state).unwrap();

        let parse_error = raw_call(&path, "{");
        assert_eq!(parse_error["jsonrpc"], "2.0");
        assert_eq!(parse_error["id"], Value::Null);
        assert_eq!(parse_error["error"]["code"], -32700);
        assert_eq!(parse_error["error"]["message"], "Parse error");

        let invalid_request = call(
            &path,
            serde_json::json!({"v": 1, "id": "old", "method": "status"}),
        );
        assert_eq!(invalid_request["id"], "old");
        assert_eq!(invalid_request["error"]["code"], -32600);
        assert_eq!(invalid_request["error"]["message"], "Invalid Request");

        let missing_method = call(
            &path,
            serde_json::json!({"jsonrpc": "2.0", "id": 3, "method": "missing"}),
        );
        assert_eq!(missing_method["id"], 3);
        assert_eq!(missing_method["error"]["code"], -32601);
        assert_eq!(missing_method["error"]["message"], "Method not found");

        let invalid_params = call(
            &path,
            serde_json::json!({"jsonrpc": "2.0", "id": 4, "method": "run", "params": {}}),
        );
        assert_eq!(invalid_params["error"]["code"], -32602);
        assert_eq!(invalid_params["error"]["message"], "Invalid params");
    }

    #[test]
    fn it_handles_json_rpc_batches() {
        let path = socket_path("batch");
        let state = Arc::new(Mutex::new(ControlState::default()));
        let targets = vec![ControlTarget {
            name: "checks".to_owned(),
            commands: vec!["cargo test".to_owned()],
        }];
        let _server = ControlServer::start_with_runner(&path, state, targets, |_| Ok(1)).unwrap();

        let response = call(
            &path,
            serde_json::json!([
                {"jsonrpc": "2.0", "id": "status", "method": "status"},
                {"jsonrpc": "2.0", "method": "status"},
                {"jsonrpc": "2.0", "id": "targets", "method": "targets"}
            ]),
        );

        assert_eq!(response.as_array().unwrap().len(), 2);
        assert_eq!(response[0]["id"], "status");
        assert_eq!(response[1]["id"], "targets");
    }

    #[test]
    fn it_creates_the_socket_parent_directory() {
        let directory =
            std::env::temp_dir().join(format!("funzzy-control-{}-nested", std::process::id()));
        let _ = std::fs::remove_dir_all(&directory);
        let path = directory.join("control.sock");
        let state = Arc::new(Mutex::new(ControlState::default()));

        let server = ControlServer::start(&path, state).unwrap();

        assert!(path.exists());
        drop(server);
        std::fs::remove_dir_all(directory).unwrap();
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
