//! `fzz config schema|example`: agent-discoverable configuration surface
//! (TASK-0057/0058). Non-interactive, side-effect-free — never reads a
//! `.watch.yaml`, starts a watcher, opens a socket, or spawns a subprocess.
//! JSON Schema is the canonical output; TOON is additive (TASK-0048).

use crate::cli::format::render_document;
use crate::cli::OutputFormat;
use crate::errors::FzzError;
use serde_json::{json, Value};

/// Schema sections (AGENT-CONFIG-CONTRACT §4).
pub const SECTIONS: [&str; 6] = ["on", "job", "matching", "execution", "parallel", "control"];

/// Example profiles (AGENT-CONFIG-CONTRACT §4).
pub const PROFILES: [&str; 3] = ["minimal", "parallel", "agent"];

/// The full deterministic JSON Schema for the preferred `jobs:` config.
fn full_schema() -> Value {
    json!({
        "$schema": "https://json-schema.org/draft/2020-12/schema",
        "$id": "funzzy://config/schema/v1",
        "title": "Funzzy .watch.yaml (preferred jobs: format)",
        "description": "Agent-discoverable configuration schema (AGENT-CONFIG-CONTRACT). Semantic checks are delegated to `fzz check`.",
        "type": "object",
        "required": ["jobs"],
        "properties": {
            "on": { "$ref": "#/$defs/on" },
            "jobs": {
                "type": "array",
                "minItems": 1,
                "items": { "$ref": "#/$defs/job" },
                "description": "Ordered list of configured jobs. Declaration order and contiguous parallel groups are semantic; jobs do not form a DAG (JOBS-CONFIG-CONTRACT)."
            }
        },
        "$defs": {
            "on": section_on(),
            "job": section_job(),
            "matching": section_matching(),
            "execution": section_execution(),
            "parallel": section_parallel(),
            "control": section_control(),
        }
    })
}

fn section_on() -> Value {
    json!({
        "type": "object",
        "title": "on",
        "description": "Common watch settings shared by every job.",
        "properties": {
            "change": { "type": ["string", "array"], "items": {"type": "string"}, "description": "Common change globs applied to all jobs." },
            "ignore": { "type": ["string", "array"], "items": {"type": "string"}, "description": "Common ignore globs; explicit config ignore always wins over gitignore." },
            "socket": { "type": "string", "description": "Control socket path; enables the control surface." },
            "concurrency": { "type": "integer", "minimum": 1, "description": "Global cap on simultaneously active tasks (default: available parallelism)." },
            "debounce": { "type": "string", "pattern": "^[0-9]+(ms|s|m)?$", "description": "Filesystem batch debounce window (default 1s)." },
            "watch_backend": { "type": "string", "enum": ["native", "poll", "auto"], "default": "auto" },
            "poll_interval": { "type": "string", "pattern": "^[0-9]+(ms|s|m)?$", "description": "Poll backend interval (default 500ms)." },
            "respect_gitignore": { "type": "boolean", "default": false, "description": "Respect workspace .gitignore rules (GITIGNORE-CONTRACT)." }
        },
        "additionalProperties": false
    })
}

fn section_job() -> Value {
    json!({
        "type": "object",
        "title": "job",
        "description": "One configured workflow unit. Runs as a task in each generation.",
        "required": ["name", "run"],
        "properties": {
            "name": { "type": "string", "description": "Stable job identity; also the runtime task name." },
            "run": { "type": ["string", "array"], "items": {"type": "string"}, "description": "Command(s); shell string or argv list." },
            "cwd": { "type": "string", "description": "Working directory for this job (relative to workspace)." },
            "env": { "type": "object", "additionalProperties": {"type": "string"}, "description": "Environment for this job. Values are never echoed in schema/examples." },
            "change": { "type": ["string", "array"], "items": {"type": "string"}, "description": "Globs that trigger this job." },
            "ignore": { "type": ["string", "array"], "items": {"type": "string"}, "description": "Globs that suppress a change match; strongest precedence." },
            "run_on_init": { "type": "boolean", "default": false, "description": "Run this job when the watcher starts." },
            "parallel": { "type": "string", "description": "Named contiguous group; members may overlap (PARALLEL-EXECUTION-CONTRACT)." }
        },
        "additionalProperties": false
    })
}

fn section_matching() -> Value {
    json!({
        "title": "matching",
        "description": "Change/ignore matching semantics (GITIGNORE-CONTRACT).",
        "properties": {
            "change_globs": { "type": "array", "items": {"type": "string"}, "description": "Glob patterns; relative and absolute forms both supported." },
            "ignore_precedence": { "type": "string", "enum": ["config-ignore-wins", "gitignore"], "description": "Explicit config ignore wins over gitignore; gitignore applies only when respect_gitignore is true." },
            "filepath_template": { "type": "string", "enum": ["{{filepath}}", "{{paths}}", "{{relative_filepath}}"], "description": "Template variables available in run commands." }
        }
    })
}

fn section_execution() -> Value {
    json!({
        "title": "execution",
        "description": "Execution policy (CLI + config).",
        "properties": {
            "on_busy": { "type": "string", "enum": ["wait", "restart"], "default": "wait" },
            "fail_fast": { "type": "boolean", "default": false },
            "sequential": { "type": "boolean", "default": false, "description": "Effective concurrency one for this run (SEQUENTIAL-OVERRIDE-CONTRACT)." },
            "log_file": { "type": "string", "description": "Mirror all output to a log file." },
            "events_file": { "type": "string", "description": "Append NDJSON run events (RUN-EVENTS-CONTRACT)." }
        }
    })
}

fn section_parallel() -> Value {
    json!({
        "title": "parallel",
        "description": "Named contiguous groups and barriers (PARALLEL-EXECUTION-CONTRACT).",
        "properties": {
            "group": { "type": "string", "description": "A job's parallel group name." },
            "concurrency": { "type": "integer", "minimum": 1, "description": "Scheduler bound (on.concurrency)." },
            "occurrence": { "type": "string", "description": "Runtime identity name#N; order inside a group is unspecified." }
        }
    })
}

fn section_control() -> Value {
    json!({
        "title": "control",
        "description": "Control socket and protocol (AGENT-FEEDBACK-CONTRACT).",
        "properties": {
            "socket": { "type": "string", "description": "on.socket path." },
            "cli_alias": { "type": "string", "enum": ["control", "ctl"], "description": "control and ctl are interchangeable." },
            "methods": { "type": "array", "items": {"type": "string"}, "description": "status, targets, run, emit, await, cancel, output, capabilities, subscribe." }
        }
    })
}

fn section_identity(section: &str) -> Value {
    json!({
        "section": section,
        "fullSchemaCommand": "fzz config schema",
        "description": format!("Bounded self-contained schema for the '{section}' section."),
    })
}

/// Handles `fzz config schema [--section S]`.
fn schema_command(section: Option<&str>, format: OutputFormat) -> Result<(), FzzError> {
    let full = full_schema();
    let document = match section {
        Some(name) => {
            let body = full["$defs"][name].clone();
            let mut doc = json!({
                "$schema": "https://json-schema.org/draft/2020-12/schema",
                "$id": format!("funzzy://config/schema/v1/sections/{name}"),
                "type": "object",
                "properties": {
                    name: body,
                },
                "description": "Section of the Funzzy .watch.yaml schema. See fullSchemaCommand for the complete schema."
            });
            let identity = section_identity(name);
            doc["section"] = identity["section"].clone();
            doc["fullSchemaCommand"] = identity["fullSchemaCommand"].clone();
            doc
        }
        None => full,
    };
    print!("{}", render_document(format, &document));
    Ok(())
}

/// The three runnable example configs; each parses through the production
/// parser and passes structural validation.
fn example_yaml(profile: &str) -> Result<String, FzzError> {
    let yaml = match profile {
        "minimal" => {
            r#"on:
  change: "**/*"

jobs:
  - name: build
    run: "cargo build"
"#
        }
        "parallel" => {
            r#"on:
  change: "src/**"
  concurrency: 2

jobs:
  - name: lint
    parallel: checks
    run: "cargo clippy"

  - name: test
    parallel: checks
    run: "cargo test"

  - name: package
    run: "cargo build"
"#
        }
        "agent" => {
            r#"# Agent loop example: control socket + a verify-style job.
# Next commands:
#   fzz check          # validate this config
#   fzz list           # see the targets
#   fzz run verify     # run once locally
#   fzz watch          # start the watcher + control socket
on:
  change: "**/*"
  concurrency: 2
  socket: .tmp/funzzy/control.sock

jobs:
  - name: verify @agent-final
    run: "cargo test"
    change: "src/**"
    ignore: "target/**"

  - name: lint @quick
    run: "cargo fmt -- --check"
    change: "src/**"
    run_on_init: true
"#
        }
        _ => unreachable!("clap validates profile"),
    };
    Ok(yaml.to_owned())
}

/// Handles `fzz config example PROFILE`.
fn example_command(profile: &str) -> Result<(), FzzError> {
    print!("{}", example_yaml(profile)?);
    Ok(())
}

/// Dispatches `fzz config`; both commands are non-interactive and
/// side-effect-free, and never read a project config.
pub fn execute_config(
    schema_section: Option<String>,
    example_profile: Option<String>,
    format: OutputFormat,
) -> Result<(), FzzError> {
    match (schema_section, example_profile) {
        // `fzz config schema` (no --section) is a full-schema request; the
        // flattened section is None in both the full and no-section cases.
        (None, None) => schema_command(None, format),
        (Some(section), None) => schema_command(Some(&section), format),
        (None, Some(profile)) => example_command(&profile),
        (Some(_), Some(_)) => unreachable!("clap rejects mixed config subcommands"),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn examples_parse_through_the_production_parser() {
        for profile in PROFILES {
            let yaml = example_yaml(profile).unwrap();
            let rules = crate::config::from_yaml(&yaml)
                .unwrap_or_else(|err| panic!("{profile} example must parse: {err:?}"));
            crate::rules::validate_rules(&rules)
                .unwrap_or_else(|err| panic!("{profile} example must validate: {err}"));
        }
    }

    #[test]
    fn full_schema_is_deterministic_and_documents_all_sections() {
        let a = full_schema();
        let b = full_schema();
        assert_eq!(
            serde_json::to_string(&a).unwrap(),
            serde_json::to_string(&b).unwrap()
        );
        for section in SECTIONS {
            assert!(a["$defs"][section].is_object(), "missing section {section}");
        }
        assert_eq!(a["properties"]["jobs"]["type"], "array");
        assert!(a["required"].as_array().unwrap().contains(&"jobs".into()));
    }

    #[test]
    fn section_schema_is_bounded_and_self_contained() {
        let doc = schema_document("parallel");
        assert_eq!(doc["section"], "parallel");
        assert_eq!(doc["fullSchemaCommand"], "fzz config schema");
        assert!(doc["properties"]["parallel"].is_object());
    }

    #[test]
    fn agent_example_has_socket_tags_and_comments() {
        let yaml = example_yaml("agent").unwrap();
        assert!(yaml.contains("socket:"), "agent example must set a socket");
        assert!(
            yaml.contains("@agent-final"),
            "agent example must have a target tag"
        );
        assert!(yaml.contains("concurrency: 2"));
        assert!(
            yaml.contains("#"),
            "agent example must carry next-command comments"
        );
    }
}

/// Returns a full section document (used by tests and the command).
fn schema_document(section: &str) -> Value {
    let full = full_schema();
    let body = full["$defs"][section].clone();
    let mut doc = json!({
        "$schema": "https://json-schema.org/draft/2020-12/schema",
        "$id": format!("funzzy://config/schema/v1/sections/{section}"),
        "type": "object",
        "properties": { section: body },
        "description": "Section of the Funzzy .watch.yaml schema.",
    });
    let identity = section_identity(section);
    doc["section"] = identity["section"].clone();
    doc["fullSchemaCommand"] = identity["fullSchemaCommand"].clone();
    doc
}
