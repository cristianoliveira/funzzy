use crate::cli::Command;
use crate::config;
use crate::control_client::{ControlClient, EmitSnapshot, StatusSnapshot, TargetSnapshot};
use crate::errors::FzzError;
use std::path::{Path, PathBuf};

/// The action a `fzz control` invocation performs. Rendering stays here;
/// transport and protocol live in `control_client`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ControlAction {
    Status,
    List,
    Run { target: String },
    Emit { path: String },
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
            ControlAction::Run { target } => {
                let generation = client
                    .run(target)
                    .map_err(|err| FzzError::GenericError(err.to_string()))?;
                print!("{}", render_run(generation));
            }
            ControlAction::Emit { path } => {
                let emit = client
                    .emit(path)
                    .map_err(|err| FzzError::GenericError(err.to_string()))?;
                print!("{}", render_emit(&emit));
            }
        }
        Ok(())
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
    }
    output
}

/// Scheduled-generation identity returned by `control run TARGET`.
pub fn render_run(generation: u64) -> String {
    format!("scheduled generation: {}\n", generation)
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
    fn render_targets_shows_count_names_and_commands() {
        let targets = vec![
            TargetSnapshot {
                name: "final checks @agent-final".to_string(),
                commands: vec!["cargo test".to_string()],
            },
            TargetSnapshot {
                name: "fast tests".to_string(),
                commands: vec!["true".to_string()],
            },
        ];
        let rendered = render_targets(&targets);
        assert!(rendered.contains("targets (2):"));
        assert!(rendered.contains("  - final checks @agent-final"));
        assert!(rendered.contains("      cargo test"));
        assert!(rendered.contains("  - fast tests"));
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
