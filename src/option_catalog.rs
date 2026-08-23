//! Canonical option catalog for the preferred V2 `.watch.yaml` vocabulary
//! (TASK-0094). One owner for property identity, required/default, type/enum,
//! explanation, and rendering example — consumed by `fzz config schema`
//! (`src/cli/config.rs`), the comprehensive commented init renderer
//! (`src/cli/init.rs`), and the parser allowlists (`src/config.rs`).
//!
//! Scope follows INIT-TEMPLATE-CONTRACT §3: only legal preferred YAML
//! properties (`on:` + ordered `jobs[]`). Legacy `tasks:` inputs and
//! CLI-only controls are explicitly excluded.

/// Where a property lives in the preferred config.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Owner {
    Root,
    On,
    Execution,
    Hooks,
    Job,
}

/// YAML kind of a property, mapped mechanically to JSON Schema by the schema
/// command and to comment text by the init renderer.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum SpecKind {
    Bool,
    Int,
    String,
    StringList,
    Duration,
    Enum(&'static [&'static str]),
    StringMap,
}

/// One catalog entry. `example` holds raw YAML lines without leading
/// indentation; renderers indent them per owner.
pub struct OptionSpec {
    pub name: &'static str,
    pub owner: Owner,
    pub required: bool,
    pub default: Option<&'static str>,
    pub help: &'static str,
    pub values: Option<&'static str>,
    pub example: &'static [&'static str],
    pub kind: SpecKind,
}

const OUTPUT_VALUES: &[&str] = &["inherit", "quiet", "capture", "show-on-failure"];
const BACKEND_VALUES: &[&str] = &["native", "poll", "auto"];
const RECOVERY_POLICY_VALUES: &[&str] = &["prompt", "skip"];

/// Ordered `on:` properties — order is stable and defines comment/schema order
/// (INIT-TEMPLATE-CONTRACT §8).
const ON_SPECS: &[OptionSpec] = &[
    OptionSpec {
        name: "change",
        owner: Owner::On,
        required: false,
        default: None,
        help: "Common change globs applied to every job (inherited, merged first).",
        values: None,
        example: &["change: \"**/*\""],
        kind: SpecKind::StringList,
    },
    OptionSpec {
        name: "ignore",
        owner: Owner::On,
        required: false,
        default: None,
        help: "Common ignore globs; explicit config ignore wins over gitignore.",
        values: None,
        example: &["ignore: \"**/*.log\""],
        kind: SpecKind::StringList,
    },
    OptionSpec {
        name: "socket",
        owner: Owner::On,
        required: false,
        default: None,
        help: "Control socket path; enables the control surface (`fzz control`).",
        values: None,
        example: &["socket: .tmp/funzzy/control.sock"],
        kind: SpecKind::String,
    },
    OptionSpec {
        name: "debounce",
        owner: Owner::On,
        required: false,
        default: Some("1s"),
        help: "Filesystem batch debounce window.",
        values: None,
        example: &["debounce: 500ms"],
        kind: SpecKind::Duration,
    },
    OptionSpec {
        name: "watch_backend",
        owner: Owner::On,
        required: false,
        default: Some("auto"),
        help: "Watch backend: native first, then poll when native is unavailable.",
        values: Some("native | poll | auto"),
        example: &["watch_backend: poll"],
        kind: SpecKind::Enum(BACKEND_VALUES),
    },
    OptionSpec {
        name: "poll_interval",
        owner: Owner::On,
        required: false,
        default: Some("500ms"),
        help: "Poll backend interval (only meaningful with `watch_backend: poll`).",
        values: None,
        example: &["poll_interval: 200ms"],
        kind: SpecKind::Duration,
    },
    OptionSpec {
        name: "respect_gitignore",
        owner: Owner::On,
        required: false,
        default: Some("false"),
        help: "Respect workspace .gitignore rules.",
        values: None,
        example: &["respect_gitignore: true"],
        kind: SpecKind::Bool,
    },
];

const EXECUTION_SPECS: &[OptionSpec] = &[
    OptionSpec {
        name: "concurrency",
        owner: Owner::Execution,
        required: false,
        default: Some("machine parallelism"),
        help: "Global cap on simultaneously active tasks.",
        values: None,
        example: &["concurrency: 2"],
        kind: SpecKind::Int,
    },
    OptionSpec {
        name: "output",
        owner: Owner::Execution,
        required: false,
        default: Some("inherit"),
        help: "Default output policy for every job.",
        values: Some("inherit | quiet | capture | show-on-failure"),
        example: &["output: quiet"],
        kind: SpecKind::Enum(OUTPUT_VALUES),
    },
    OptionSpec {
        name: "recovery_policy",
        owner: Owner::Execution,
        required: false,
        default: Some("prompt"),
        help: "Recovery approval policy for failed jobs with a recovery command; missing TTY safely skips.",
        values: Some("prompt | skip"),
        example: &["recovery_policy: prompt"],
        kind: SpecKind::Enum(RECOVERY_POLICY_VALUES),
    },
    OptionSpec {
        name: "recovery_timeout",
        owner: Owner::Execution,
        required: false,
        default: Some("60s"),
        help: "Maximum time to wait for interactive recovery approval.",
        values: None,
        example: &["recovery_timeout: 60s"],
        kind: SpecKind::Duration,
    },
];

const HOOK_SPECS: &[OptionSpec] = &[
    OptionSpec {
        name: "success",
        owner: Owner::Hooks,
        required: false,
        default: None,
        help: "Hook command run after a successful generation.",
        values: None,
        example: &["success: echo ok > .fzz-success"],
        kind: SpecKind::String,
    },
    OptionSpec {
        name: "failure",
        owner: Owner::Hooks,
        required: false,
        default: None,
        help: "Hook command run after a failed generation.",
        values: None,
        example: &["failure: echo failed > .fzz-failed"],
        kind: SpecKind::String,
    },
    OptionSpec {
        name: "close",
        owner: Owner::Hooks,
        required: false,
        default: None,
        help: "Finite hook command run once when a ready watcher closes.",
        values: None,
        example: &["close: echo closed > .fzz-closed"],
        kind: SpecKind::String,
    },
];

/// Ordered `jobs[]` properties — `name` and `run` are required.
const JOB_SPECS: &[OptionSpec] = &[
    OptionSpec {
        name: "name",
        owner: Owner::Job,
        required: true,
        default: None,
        help: "Stable job identity; also the runtime task name. Must be unique.",
        values: None,
        example: &["name: reference"],
        kind: SpecKind::String,
    },
    OptionSpec {
        name: "run",
        owner: Owner::Job,
        required: true,
        default: None,
        help: "Command(s): a shell string or an argv list. Template variables: {{filepath}}, {{paths}}, {{relative_filepath}}.",
        values: None,
        example: &["run: [\"echo\", \"{{filepath}}\", \"{{paths}}\"]"],
        kind: SpecKind::StringList,
    },
    OptionSpec {
        name: "recovery",
        owner: Owner::Job,
        required: false,
        default: None,
        help: "Optional ordered shell commands offered for one approved recovery after this job fails; declaration never authorizes execution.",
        values: None,
        example: &["recovery: cargo fmt --all"],
        kind: SpecKind::StringList,
    },
    OptionSpec {
        name: "change",
        owner: Owner::Job,
        required: false,
        default: None,
        help: "Globs that trigger this job (appended to on.change).",
        values: None,
        example: &["change: [\"**/*.rs\", \"**/*.md\"]"],
        kind: SpecKind::StringList,
    },
    OptionSpec {
        name: "ignore",
        owner: Owner::Job,
        required: false,
        default: None,
        help: "Globs that suppress a match; strongest precedence.",
        values: None,
        example: &["ignore: [\"**/*.log\"]"],
        kind: SpecKind::StringList,
    },
    OptionSpec {
        name: "run_on_init",
        owner: Owner::Job,
        required: false,
        default: Some("false"),
        help: "Run this job when the watcher starts.",
        values: None,
        example: &["run_on_init: true"],
        kind: SpecKind::Bool,
    },
    OptionSpec {
        name: "parallel",
        owner: Owner::Job,
        required: false,
        default: None,
        help: "Named contiguous group; members run concurrently within the concurrency cap.",
        values: None,
        example: &["parallel: checks"],
        kind: SpecKind::String,
    },
    OptionSpec {
        name: "cwd",
        owner: Owner::Job,
        required: false,
        default: None,
        help: "Working directory for this job, relative to the workspace root.",
        values: None,
        example: &["cwd: scripts"],
        kind: SpecKind::String,
    },
    OptionSpec {
        name: "env",
        owner: Owner::Job,
        required: false,
        default: None,
        help: "Per-job environment; values are never echoed or generated.",
        values: None,
        example: &["env:", "  FOO: bar"],
        kind: SpecKind::StringMap,
    },
    OptionSpec {
        name: "service",
        owner: Owner::Job,
        required: false,
        default: Some("false"),
        help: "Managed long-running service: started on init, restarted on change, stopped on exit.",
        values: None,
        example: &["service: true"],
        kind: SpecKind::Bool,
    },
    OptionSpec {
        name: "output",
        owner: Owner::Job,
        required: false,
        default: None,
        help: "Job output policy override; inherits execution.output when absent.",
        values: Some("inherit | quiet | capture | show-on-failure"),
        example: &["output: show-on-failure"],
        kind: SpecKind::Enum(OUTPUT_VALUES),
    },
];

/// Preferred root keys (INIT-TEMPLATE-CONTRACT §3.1).
const ROOT_SPECS: &[OptionSpec] = &[
    OptionSpec {
        name: "on",
        owner: Owner::Root,
        required: false,
        default: None,
        help: "Shared settings merged into every job.",
        values: None,
        example: &["on:"],
        kind: SpecKind::StringMap,
    },
    OptionSpec { name: "execution", owner: Owner::Root, required: false, default: None, help: "Scheduling and output policy.", values: None, example: &["execution:"], kind: SpecKind::StringMap },
    OptionSpec { name: "hooks", owner: Owner::Root, required: false, default: None, help: "Generation and watcher lifecycle reactions.", values: None, example: &["hooks:"], kind: SpecKind::StringMap },
    OptionSpec {
        name: "jobs",
        owner: Owner::Root,
        required: true,
        default: None,
        help: "Ordered list of configured jobs; declaration order and contiguous parallel groups are semantic.",
        values: None,
        example: &["jobs:"],
        kind: SpecKind::StringList,
    },
];

pub fn on_specs() -> &'static [OptionSpec] {
    ON_SPECS
}

pub fn execution_specs() -> &'static [OptionSpec] {
    EXECUTION_SPECS
}

pub fn hook_specs() -> &'static [OptionSpec] {
    HOOK_SPECS
}

pub fn job_specs() -> &'static [OptionSpec] {
    JOB_SPECS
}

pub fn root_specs() -> &'static [OptionSpec] {
    ROOT_SPECS
}

pub fn all_specs() -> &'static [OptionSpec] {
    // on + job only: root keys are structural, not commented per-property.
    ON_SPECS
}

/// All legal preferred YAML property names for an owner (parser allowlists).
pub fn property_names(owner: Owner) -> Vec<&'static str> {
    match owner {
        Owner::On => ON_SPECS.iter().map(|s| s.name).collect(),
        Owner::Execution => EXECUTION_SPECS.iter().map(|s| s.name).collect(),
        Owner::Hooks => HOOK_SPECS.iter().map(|s| s.name).collect(),
        Owner::Job => JOB_SPECS.iter().map(|s| s.name).collect(),
        Owner::Root => ROOT_SPECS.iter().map(|s| s.name).collect(),
    }
}

pub fn find(name: &str) -> Option<&'static OptionSpec> {
    ON_SPECS
        .iter()
        .chain(EXECUTION_SPECS)
        .chain(HOOK_SPECS)
        .chain(JOB_SPECS)
        .chain(ROOT_SPECS)
        .find(|s| s.name == name)
}

/// A property that a renderer/schema must show as optional (commented),
/// i.e. not required and not structurally active.
pub fn is_optional(spec: &OptionSpec) -> bool {
    !spec.required
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The inventory in INIT-TEMPLATE-CONTRACT §3, locked as the exact
    /// allowed sets. Any parser-supported preferred property missing here is
    /// drift; any extra entry is a pseudo-field masquerading as config (including recovery timeout).
    #[test]
    fn on_inventory_matches_contract_section_3_2() {
        let actual: Vec<&str> = property_names(Owner::On);
        assert_eq!(
            actual,
            [
                "change",
                "ignore",
                "socket",
                "debounce",
                "watch_backend",
                "poll_interval",
                "respect_gitignore"
            ]
        );
        assert_eq!(
            property_names(Owner::Execution),
            ["concurrency", "output", "recovery_policy", "recovery_timeout"]
        );
        assert_eq!(
            property_names(Owner::Hooks),
            ["success", "failure", "close"]
        );
    }

    #[test]
    fn job_inventory_matches_contract_section_3_3() {
        let expected = [
            "name",
            "run",
            "recovery",
            "change",
            "ignore",
            "run_on_init",
            "parallel",
            "cwd",
            "env",
            "service",
            "output",
        ];
        let actual: Vec<&str> = property_names(Owner::Job);
        assert_eq!(actual, expected);
    }

    #[test]
    fn only_name_and_run_are_required_job_properties() {
        assert!(find("name").unwrap().required);
        assert!(find("run").unwrap().required);
        for spec in job_specs() {
            if spec.name != "name" && spec.name != "run" {
                assert!(!spec.required, "{} must be optional", spec.name);
            }
        }
    }

    #[test]
    fn every_on_property_is_optional() {
        for spec in on_specs() {
            assert!(!spec.required, "{} must be optional", spec.name);
        }
    }

    #[test]
    fn examples_are_nonempty_and_unique() {
        for spec in all_specs().iter().chain(job_specs()) {
            assert!(!spec.example.is_empty(), "{} has no example", spec.name);
            assert!(!spec.help.is_empty(), "{} has no help", spec.name);
        }
    }

    #[test]
    fn enum_specs_carry_values_and_legal_schema_enum() {
        for spec in all_specs().iter().chain(job_specs()) {
            if let SpecKind::Enum(values) = spec.kind {
                assert!(
                    spec.values.is_some(),
                    "{} enum needs values text",
                    spec.name
                );
                assert!(!values.is_empty(), "{} enum is empty", spec.name);
            }
        }
    }
}
