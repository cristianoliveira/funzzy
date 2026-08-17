//! `fzz config schema|example`: agent-discoverable configuration surface
//! (TASK-0057/0058). Non-interactive, side-effect-free — never reads a
//! `.watch.yaml`, starts a watcher, opens a socket, or spawns a subprocess.
//! JSON Schema is the canonical output; TOON is additive (TASK-0048).

use crate::cli::format::render_document;
use crate::cli::OutputFormat;
use crate::errors::FzzError;
use crate::option_catalog::{self, OptionSpec, Owner, SpecKind};
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

/// Maps one catalog property to its JSON Schema fragment (TASK-0094: the
/// schema and the init renderer consume the same option metadata).
fn schema_property(spec: &OptionSpec) -> Value {
    let mut prop = match spec.kind {
        SpecKind::Bool => json!({ "type": "boolean" }),
        SpecKind::Int => json!({ "type": "integer", "minimum": 1 }),
        SpecKind::String => json!({ "type": "string" }),
        SpecKind::StringList => json!({
            "type": ["string", "array"],
            "items": { "type": "string" }
        }),
        SpecKind::Duration => json!({
            "type": "string",
            "pattern": "^[0-9]+(ms|s|m)?$"
        }),
        SpecKind::Enum(values) => json!({ "type": "string", "enum": values }),
        SpecKind::StringMap => json!({
            "type": "object",
            "additionalProperties": { "type": "string" }
        }),
    };
    prop["description"] = json!(spec.help);
    // Literal defaults only: textual defaults like "machine parallelism" are
    // guidance for comments, not schema values.
    if let Some(default) = spec.default {
        match spec.kind {
            SpecKind::Bool => prop["default"] = json!(default == "true"),
            SpecKind::Enum(_) | SpecKind::String | SpecKind::Duration => {
                prop["default"] = json!(default)
            }
            _ => {}
        }
    }
    prop
}

/// Structural properties for a section, driven by the canonical catalog.
fn section_properties(owner: Owner) -> Value {
    let mut props = serde_json::Map::new();
    for spec in match owner {
        Owner::On => option_catalog::on_specs(),
        Owner::Job => option_catalog::job_specs(),
        Owner::Root => option_catalog::root_specs(),
    } {
        props.insert(spec.name.to_string(), schema_property(spec));
    }
    Value::Object(props)
}

fn section_on() -> Value {
    json!({
        "type": "object",
        "title": "on",
        "description": "Common watch settings shared by every job.",
        "properties": section_properties(Owner::On),
        "additionalProperties": false
    })
}

fn section_job() -> Value {
    json!({
        "type": "object",
        "title": "job",
        "description": "One configured workflow unit. Runs as a task in each generation.",
        "required": ["name", "run"],
        "properties": section_properties(Owner::Job),
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
pub(crate) fn example_yaml(profile: &str) -> Result<String, FzzError> {
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

/// Returns a full section document used by focused schema tests.
#[cfg(test)]
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

#[cfg(test)]
mod catalog_parity_tests {
    use super::*;
    use crate::option_catalog::{self, Owner};

    fn prop_names(section: &str) -> Vec<String> {
        let full = full_schema();
        full["$defs"][section]["properties"]
            .as_object()
            .expect("section properties object")
            .keys()
            .map(|k| k.to_string())
            .collect()
    }

    /// TASK-0094: structural schema properties for on/job must exactly equal
    /// the canonical catalog — no accepted field missing, no pseudo-field.
    #[test]
    fn on_section_matches_catalog_exactly() {
        let mut expected: Vec<String> = option_catalog::property_names(Owner::On)
            .into_iter()
            .map(str::to_string)
            .collect();
        expected.sort();
        let mut actual = prop_names("on");
        actual.sort();
        assert_eq!(actual, expected);
        assert!(actual.contains(&"success".to_string()));
        assert!(actual.contains(&"failure".to_string()));
        assert!(actual.contains(&"output".to_string()));
    }

    #[test]
    fn job_section_matches_catalog_exactly() {
        let mut expected: Vec<String> = option_catalog::property_names(Owner::Job)
            .into_iter()
            .map(str::to_string)
            .collect();
        expected.sort();
        let mut actual = prop_names("job");
        actual.sort();
        assert_eq!(actual, expected);
        assert!(actual.contains(&"service".to_string()));
        assert!(actual.contains(&"output".to_string()));
    }

    /// Conceptual sections (matching/execution/parallel/control) must stay
    /// separate and never leak into structural on/job definitions. `parallel`
    /// is a real job property, so the pseudo-fields are matching/execution/control.
    #[test]
    fn conceptual_sections_do_not_masquerade_as_config_keys() {
        for section in ["on", "job"] {
            for prop in prop_names(section) {
                assert!(
                    !["matching", "execution", "control"].contains(&prop.as_str()),
                    "{prop} leaked into {section} structural properties"
                );
            }
        }
    }
}
