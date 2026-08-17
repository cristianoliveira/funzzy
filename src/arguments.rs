//! Clap-backed CLI argument parsing.
//!
//! Owns parser details and exposes semantic action/option types to
//! application code. V2 structure (TASK-0015): real Clap subcommands
//! (`init`, `watch`, `run`, `exec`); `fzz` with no subcommand is configured watch.
//! `-V`/`--version` is Clap's built-in version flag (stdout, exit 0);
//! `-v`/`--verbose` is the verbose flag; parse errors use Clap's native
//! handling (stderr, exit 2).

use clap::{Arg, ArgAction, Command};

use crate::cli::{ControlAction, OutputFormat};
use std::time::Duration;

/// Busy-run policy: what to do when a change arrives while a run is active.
/// Replaces the V1 `--non-block` flag (TASK-0018).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum OnBusy {
    /// Finish the active run before processing the next change.
    Wait,
    /// Cancel the active child and schedule the newest generation.
    Restart,
}

/// Semantic application action selected from the parsed subcommand.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Action {
    /// `fzz` (no subcommand) or `fzz watch [TARGET]`: run configured tasks.
    Watch { target: Option<String> },
    /// `fzz list`: print configured tasks.
    List,
    /// `fzz check`: validate configuration without starting a watcher.
    Check,
    /// `fzz completions SHELL`: emit shell completion scripts.
    Completions { shell: String },
    /// `fzz config schema|example`: agent-discoverable config surface.
    Config {
        /// `schema` (Some(section) when --section given) vs `example`.
        schema_section: Option<Option<String>>,
        example_profile: Option<String>,
        /// Structured output format for schema (TASK-0048/0058).
        format: OutputFormat,
    },
    /// `fzz run TARGET`: execute selected configured tasks once, locally.
    Run { target: String },
    /// `fzz explain PATH`: print which tasks a path matches or is ignored by.
    Explain { path: String },
    /// `fzz init [--migrate]`: create or migrate the default config file.
    Init,
    /// `fzz control status|list|run TARGET`: talk to a running watcher.
    Control {
        action: ControlAction,
        /// `control --socket <PATH>` override.
        socket: Option<String>,
        /// `control --format toon|json|human`: structured output selection.
        format: OutputFormat,
    },
    /// `fzz exec -- PROGRAM ARG...`: ad-hoc command over stdin-supplied paths.
    Exec { command: Vec<String> },
}

/// Parser-owned, semantic application arguments.
#[derive(Debug, Clone)]
pub struct Arguments {
    pub action: Action,
    pub config: Option<String>,
    pub log_truncate_on_change: bool,
    pub log_file: Option<String>,
    /// NDJSON run-event stream destination (TASK-0039); None = no stream.
    pub events_file: Option<String>,
    pub control_socket: Option<String>,
    pub migrate: bool,
    pub on_busy: OnBusy,
    pub no_run_on_init: bool,
    pub fail_fast: bool,
    pub sequential: bool,
    pub verbose: bool,
}

fn parse_config_format(raw: Option<&str>) -> OutputFormat {
    match raw.unwrap_or("human") {
        "toon" => OutputFormat::Toon,
        "json" => OutputFormat::Json,
        _ => OutputFormat::Human,
    }
}

impl Arguments {
    /// Parse process arguments, handing help/version/error display to Clap:
    /// help and version print to stdout and exit 0; parse errors print to
    /// stderr and exit 2.
    pub fn parse() -> Arguments {
        Self::try_parse_from(std::env::args()).unwrap_or_else(|err| err.exit())
    }

    /// Parse from an argument iterator without exiting (used by unit tests).
    fn try_parse_from<I, T>(argv: I) -> Result<Arguments, clap::Error>
    where
        I: IntoIterator<Item = T>,
        T: Into<std::ffi::OsString> + Clone,
    {
        let matches = command().try_get_matches_from(argv)?;

        let config = matches
            .get_one::<String>("config")
            .cloned()
            .filter(|value| !value.is_empty());
        let log_file = matches.get_one::<String>("log_file").cloned();
        let events_file = matches.get_one::<String>("events_file").cloned();
        let control_socket = matches.get_one::<String>("control_socket").cloned();

        let (action, migrate) = match matches.subcommand() {
            None => (Action::Watch { target: None }, false),
            Some(("watch", sub)) => {
                let target = sub.get_one::<String>("target").cloned();
                (Action::Watch { target }, false)
            }
            Some(("list", _)) => (Action::List, false),
            Some(("check", _)) => (Action::Check, false),
            Some(("completions", sub)) => (
                Action::Completions {
                    shell: sub
                        .get_one::<String>("shell")
                        .cloned()
                        .expect("shell is required by clap"),
                },
                false,
            ),
            Some(("config", sub)) => {
                let action = match sub.subcommand() {
                    Some(("schema", schema_sub)) => {
                        let format = parse_config_format(
                            schema_sub
                                .get_one::<String>("config_format")
                                .map(String::as_str),
                        );
                        Action::Config {
                            schema_section: Some(schema_sub.get_one::<String>("section").cloned()),
                            example_profile: None,
                            format,
                        }
                    }
                    Some(("example", example_sub)) => {
                        let format = parse_config_format(
                            example_sub
                                .get_one::<String>("config_format")
                                .map(String::as_str),
                        );
                        Action::Config {
                            schema_section: None,
                            example_profile: example_sub.get_one::<String>("profile").cloned(),
                            format,
                        }
                    }
                    _ => unreachable!("clap rejects unknown config subcommand"),
                };
                (action, false)
            }
            Some(("run", sub)) => {
                let target = sub
                    .get_one::<String>("target")
                    .cloned()
                    .expect("target is required by clap");
                (Action::Run { target }, false)
            }
            Some(("explain", sub)) => {
                let path = sub
                    .get_one::<String>("path")
                    .cloned()
                    .expect("path is required by clap");
                (Action::Explain { path }, false)
            }
            Some(("init", sub)) => (Action::Init, sub.get_flag("migrate")),
            Some(("control", sub)) => {
                let socket = sub.get_one::<String>("socket").cloned();
                let format = match sub
                    .get_one::<String>("format")
                    .map(String::as_str)
                    .unwrap_or("human")
                {
                    "toon" => OutputFormat::Toon,
                    "json" => OutputFormat::Json,
                    _ => OutputFormat::Human,
                };
                let action = match sub.subcommand() {
                    Some(("status", _)) => ControlAction::Status,
                    Some(("list", _)) => ControlAction::List,
                    Some(("capabilities", _)) => ControlAction::Capabilities,
                    Some(("config", _)) => ControlAction::Config,
                    Some(("run", run_sub)) => ControlAction::Run {
                        target: run_sub
                            .get_one::<String>("target")
                            .cloned()
                            .expect("target is required by clap"),
                        wait: run_sub.get_flag("wait"),
                        timeout: run_sub.get_one::<Duration>("timeout").copied(),
                        sequential: run_sub.get_flag("sequential"),
                    },
                    Some(("emit", emit_sub)) => ControlAction::Emit {
                        path: emit_sub
                            .get_one::<String>("path")
                            .cloned()
                            .expect("path is required by clap"),
                        wait: emit_sub.get_flag("wait"),
                        timeout: emit_sub.get_one::<Duration>("timeout").copied(),
                    },
                    Some(("await", await_sub)) => ControlAction::Await {
                        after: await_sub.get_one::<u64>("after").copied(),
                        generation: await_sub.get_one::<u64>("generation").copied(),
                        timeout: await_sub
                            .get_one::<Duration>("timeout")
                            .copied()
                            .expect("timeout is required by clap"),
                    },
                    Some(("cancel", cancel_sub)) => ControlAction::Cancel {
                        generation: cancel_sub
                            .get_one::<u64>("generation")
                            .copied()
                            .expect("generation is required by clap"),
                        wait: cancel_sub.get_flag("wait"),
                        timeout: cancel_sub.get_one::<Duration>("timeout").copied(),
                    },
                    Some(("output", output_sub)) => ControlAction::Output {
                        generation: output_sub
                            .get_one::<u64>("generation")
                            .copied()
                            .expect("generation is required by clap"),
                        task: output_sub.get_one::<String>("task").cloned(),
                        stream: if output_sub.get_flag("stdout") {
                            Some("stdout".to_string())
                        } else if output_sub.get_flag("stderr") {
                            Some("stderr".to_string())
                        } else {
                            None
                        },
                        tail: output_sub.get_one::<u64>("tail").copied(),
                        full: output_sub.get_flag("full"),
                        page: output_sub.get_flag("page"),
                        page_size: output_sub.get_one::<u64>("page-size").copied(),
                        cursor: output_sub.get_one::<String>("cursor").cloned(),
                    },
                    _ => unreachable!("clap rejects unknown control subcommand before dispatch"),
                };
                (
                    Action::Control {
                        action,
                        socket,
                        format,
                    },
                    false,
                )
            }
            Some(("exec", sub)) => {
                let command: Vec<String> = sub
                    .get_many::<String>("command")
                    .map(|values| values.cloned().collect())
                    .unwrap_or_default();
                (Action::Exec { command }, false)
            }
            Some((other, _)) => {
                unreachable!("clap rejects unknown subcommand {other:?} before dispatch")
            }
        };

        Ok(Arguments {
            action,
            config,
            log_truncate_on_change: matches.get_flag("log_truncate_on_change"),
            log_file,
            events_file,
            control_socket,
            migrate,
            on_busy: if matches.get_flag("restart") {
                OnBusy::Restart
            } else {
                match matches
                    .get_one::<String>("on_busy")
                    .map(String::as_str)
                    .unwrap_or("wait")
                {
                    "restart" => OnBusy::Restart,
                    _ => OnBusy::Wait,
                }
            },
            no_run_on_init: matches.get_flag("no_run_on_init"),
            fail_fast: matches.get_flag("fail_fast"),
            sequential: match matches.subcommand() {
                Some((name, sub)) if matches!(name, "run" | "watch") => sub.get_flag("sequential"),
                _ => false,
            },
            verbose: matches.get_flag("verbose"),
        })
    }

    pub fn help_text() -> String {
        command().render_help().to_string()
    }
}

pub fn command() -> Command {
    Command::new("funzzy")
        .about("Funzzy the watcher.\n\nAlias:\n  fzz -> funzzy")
        .version(env!("CARGO_PKG_VERSION"))
        .disable_version_flag(true)
        .help_template(HELP_TEMPLATE)
        .subcommand_required(false)
        .arg(
            Arg::new("version")
                .short('V')
                .long("version")
                .global(true)
                .action(ArgAction::Version)
                .help("Show version."),
        )
        .arg(
            Arg::new("config")
                .short('c')
                .long("config")
                .global(true)
                .value_name("cfgfile")
                .value_parser(clap::builder::ValueParser::string())
                .help("Use given config file."),
        )
        .arg(
            Arg::new("on_busy")
                .long("on-busy")
                .global(true)
                .value_name("POLICY")
                .value_parser(clap::builder::PossibleValuesParser::new([
                    "wait", "restart",
                ]))
                .default_value("wait")
                .help("What to do when a change arrives while a run is active (wait|restart)."),
        )
        .arg(
            Arg::new("restart")
                .long("restart")
                .global(true)
                .action(ArgAction::SetTrue)
                .help("Convenience alias for --on-busy restart."),
        )
        .arg(
            Arg::new("fail_fast")
                .short('b')
                .long("fail-fast")
                .global(true)
                .action(ArgAction::SetTrue)
                .help("Bail current execution if a task fails (exit code != 0)."),
        )
        .arg(
            Arg::new("log_truncate_on_change")
                .short('T')
                .long("log-truncate-on-change")
                .global(true)
                .action(ArgAction::SetTrue)
                .help("Truncate the log file when the config reloads (requires --log-file)."),
        )
        .arg(
            Arg::new("log_file")
                .short('l')
                .long("log-file")
                .global(true)
                .value_name("file")
                .value_parser(clap::builder::ValueParser::string())
                .help("Write all output to the specified log file in addition to the console."),
        )
        .arg(
            Arg::new("events_file")
                .long("events")
                .global(true)
                .value_name("file")
                .value_parser(clap::builder::ValueParser::string())
                .help("Append NDJSON run events (started/tick/task_terminal/finished/cancelled) to the specified file (TASK-0039)."),
        )
        .arg(
            Arg::new("control_socket")
                .long("control-socket")
                .global(true)
                .value_name("path")
                .value_parser(clap::builder::ValueParser::string())
                .help("Expose watcher status over a Unix socket (implies --on-busy restart)."),
        )
        .arg(
            Arg::new("no_run_on_init")
                .long("no-run-on-init")
                .global(true)
                .action(ArgAction::SetTrue)
                .help("Do not run tasks on initialization."),
        )
        .arg(
            Arg::new("verbose")
                .short('v')
                .long("verbose")
                .global(true)
                .action(ArgAction::SetTrue)
                .help("Use verbose output."),
        )
        .subcommand(
            Command::new("init")
                .about("Create or migrate a '.watch.yaml' file.")
                .version(env!("CARGO_PKG_VERSION"))
                .arg(
                    Arg::new("migrate")
                        .long("migrate")
                        .action(ArgAction::SetTrue)
                        .help("Migrate legacy root task list to current format."),
                ),
        )
        .subcommand(
            Command::new("watch")
                .about("Watch for file changes and run configured tasks.")
                .version(env!("CARGO_PKG_VERSION"))
                .arg(
                    Arg::new("sequential")
                        .long("sequential")
                        .action(ArgAction::SetTrue)
                        .help("Run tasks with effective concurrency 1 for this watch session."),
                )
                .arg(
                    Arg::new("target")
                        .value_name("TARGET")
                        .num_args(0..=1)
                        .value_parser(clap::builder::ValueParser::string())
                        .help("Optional task name/@tag substring; only matching tasks run."),
                ),
        )
        .subcommand(
            Command::new("list")
                .about("List configured tasks.")
                .version(env!("CARGO_PKG_VERSION")),
        )
        .subcommand(
            Command::new("run")
                .about("Run configured tasks once in this process (no watcher or control socket).")
                .long_about(
                    "Run configured tasks once in this process, then exit with their combined outcome.\n\nThis is local execution. `fzz control run TARGET` requests work from an existing watcher. Path filtering is not supported; TARGET selects a full configured workflow.",
                )
                .version(env!("CARGO_PKG_VERSION"))
                .arg(
                    Arg::new("sequential")
                        .long("sequential")
                        .action(ArgAction::SetTrue)
                        .help("Run the selected workflow with effective concurrency 1."),
                )
                .arg(
                    Arg::new("target")
                        .value_name("TARGET")
                        .num_args(1)
                        .required(true)
                        .value_parser(clap::builder::ValueParser::string())
                        .help("Exact task name, @tag, or unambiguous name substring."),
                ),
        )
        .subcommand(
            Command::new("check")
                .about("Validate configuration without starting a watcher.")
                .version(env!("CARGO_PKG_VERSION"))
                .long_about(
                    "Load and validate the configured workflow: schema, globs, durations, concurrency, parallel groups, and paths. Never starts a watcher, executes a command, or opens a socket. Exit 0 when valid; non-zero with actionable errors when not.",
                ),
        )
        .subcommand(
            Command::new("completions")
                .about("Generate shell completion scripts.")
                .version(env!("CARGO_PKG_VERSION"))
                .long_about(
                    "Generate shell completion scripts for bash, zsh, fish, elvish, or powershell.\n\nExamples:\n  fzz completions bash > /etc/bash_completion.d/fzz\n  fzz completions zsh > ${fpath[1]}/_fzz\n  fzz completions fish > ~/.config/fish/completions/fzz.fish",
                )
                .arg(
                    Arg::new("shell")
                        .value_name("SHELL")
                        .num_args(1)
                        .required(true)
                        .value_parser(clap::builder::PossibleValuesParser::new(["bash", "zsh", "fish", "elvish", "powershell"]))
                        .help("Shell to generate completions for."),
                ),
        )
        .subcommand(
            Command::new("config")
                .about("Describe configuration structure and print runnable examples.")
                .version(env!("CARGO_PKG_VERSION"))
                .subcommand_required(true)
                .arg_required_else_help(true)
                .subcommand(
                    Command::new("schema")
                        .version(env!("CARGO_PKG_VERSION"))
                        .about("Print the JSON Schema for the preferred jobs: config.")
                        .long_about(
                            "Print a deterministic JSON Schema describing the preferred ordered `jobs:` .watch.yaml format. Exit 0. No config, watcher, or socket is read.\n\nOutput contract: one JSON Schema document on stdout, nothing else. `--section` returns a bounded self-contained section plus its identity and a hint for the full schema.\n\nExamples:\n  fzz config schema\n  fzz config schema --section parallel\n  fzz config schema --section job --format toon",
                        )
                        .arg(
                            Arg::new("section")
                                .long("section")
                                .value_name("SECTION")
                                .value_parser(clap::builder::PossibleValuesParser::new([
                                    "on", "job", "matching", "execution", "parallel", "control",
                                ]))
                                .help("Emit one bounded schema section (on|job|matching|execution|parallel|control)."),
                        )
                        .arg(
                            Arg::new("config_format")
                                .long("format")
                                .value_name("FORMAT")
                                .value_parser(clap::builder::PossibleValuesParser::new(["toon", "json", "human"]))
                                .help("Structured output format: toon, json, or human (default)."),
                        ),
                )
                .subcommand(
                    Command::new("example")
                        .version(env!("CARGO_PKG_VERSION"))
                        .about("Print a runnable .watch.yaml example to stdout.")
                        .long_about(
                            "Print a valid runnable .watch.yaml to stdout with no prose mixed in. The output parses through the same production parser. Exit 0. No config, watcher, or socket is read.\n\nExamples:\n  fzz config example minimal\n  fzz config example parallel\n  fzz config example agent",
                        )
                        .arg(
                            Arg::new("profile")
                                .value_name("PROFILE")
                                .num_args(1)
                                .required(true)
                                .value_parser(clap::builder::PossibleValuesParser::new(["minimal", "parallel", "agent"]))
                                .help("Example profile: minimal, parallel, or agent."),
                        )
                        .arg(
                            Arg::new("config_format")
                                .long("format")
                                .value_name("FORMAT")
                                .value_parser(clap::builder::PossibleValuesParser::new(["toon", "json", "human"]))
                                .help("Structured output format: toon, json, or human (default)."),
                        ),
                ),
        )
        .subcommand(
            Command::new("explain")
                .about("Explain which configured tasks a path matches or is ignored by.")
                .version(env!("CARGO_PKG_VERSION"))
                .arg(
                    Arg::new("path")
                        .value_name("PATH")
                        .num_args(1)
                        .required(true)
                        .value_parser(clap::builder::ValueParser::string())
                        .help("Path to explain (relative or absolute)."),
                ),
        )
        .subcommand(
            Command::new("exec")
                .about("Run an ad-hoc command over stdin-supplied paths.")
                .version(env!("CARGO_PKG_VERSION"))
                .arg(
                    Arg::new("command")
                        .value_name("COMMAND")
                        .num_args(1..)
                        .required(true)
                        .trailing_var_arg(true)
                        .allow_hyphen_values(true)
                        .value_parser(clap::builder::ValueParser::string())
                        .help("Program and arguments to run on each change."),
                ),
        )
        .subcommand(
            Command::new("control")
                .visible_alias("ctl")
                .about("Interact with a running watcher over its control socket.")
                .version(env!("CARGO_PKG_VERSION"))
                .subcommand_required(true)
                .arg_required_else_help(true)
                .arg(
                    Arg::new("socket")
                        .long("socket")
                        .value_name("PATH")
                        .value_parser(clap::builder::ValueParser::string())
                        .help("Control socket path (overrides --control-socket and on.socket)."),
                )
                .arg(
                    Arg::new("format")
                        .long("format")
                        .value_name("FORMAT")
                        .value_parser(clap::builder::PossibleValuesParser::new(["toon", "json", "human"]))
                        .help("Structured output format: toon (default for non-TTY), json, or human (default for TTY)."),
                )
                .subcommand(
                    Command::new("status")
                        .about("Print the running watcher's state.")
                        .version(env!("CARGO_PKG_VERSION")),
                )
                .subcommand(
                    Command::new("list")
                        .about("List targets from the running watcher.")
                        .version(env!("CARGO_PKG_VERSION")),
                )
                .subcommand(
                    Command::new("capabilities")
                        .about("Print the running watcher's protocol capabilities.")
                        .version(env!("CARGO_PKG_VERSION"))
                        .long_about(
                            "Print protocol facts about the running watcher: protocol/schema/watcher versions, supported methods, optional fields, output formats, limits, and feature flags.\n\nThese are static negotiation facts, not dynamic watcher state; the request performs no config reload or filesystem scan and works whether the watcher is idle or busy.",
                        ),
                )
                .subcommand(
                    Command::new("config")
                        .about("Print the running watcher's config lifecycle.")
                        .version(env!("CARGO_PKG_VERSION"))
                        .long_about(
                            "Print the config lifecycle from the shared state source: the current phase (idle, configReloading, configReloaded, or terminal configInvalid), the revision facts, and the bounded transition history.\n\nA valid hot reload preserves the instance identity; a formatting-only save is quiet here (no transition) and only the watcher log reports it.",
                        ),
                )
                .subcommand(
                    Command::new("run")
                        .about("Trigger a named target on the running watcher and report the scheduled generation.")
                        .version(env!("CARGO_PKG_VERSION"))
                        .long_about(
                            "Trigger a named target on the running watcher and report the scheduled generation.\n\nThis is remote execution: an existing watcher owns the work. Local execution is `fzz run TARGET`. With `--wait` the exact scheduled generation is awaited atomically and its terminal observation is returned in one round trip.",
                        )
                        .arg(
                            Arg::new("target")
                                .value_name("TARGET")
                                .num_args(1)
                                .required(true)
                                .value_parser(clap::builder::ValueParser::string())
                                .help("Exact task name, @tag, or name substring on the running watcher."),
                        )
                        .arg(
                            Arg::new("wait")
                                .long("wait")
                                .action(clap::ArgAction::SetTrue)
                                .requires("timeout")
                                .help("Await the scheduled generation to terminal and return its observation."),
                        )
                        .arg(
                            Arg::new("sequential")
                                .long("sequential")
                                .action(clap::ArgAction::SetTrue)
                                .help("Run this exact generation with effective concurrency one (TASK-0073)."),
                        )
                        .arg(
                            Arg::new("timeout")
                                .long("timeout")
                                .value_name("DURATION")
                                .requires("wait")
                                .value_parser(clap::builder::ValueParser::new(crate::cli::control::parse_duration))
                                .help("Bound for the await: <number> seconds, or <number>ms/s/m (required with --wait)."),
                        ),
                )
                .subcommand(
                    Command::new("emit")
                        .about("Report a synthetic path change to the running watcher.")
                        .version(env!("CARGO_PKG_VERSION"))
                        .long_about(
                            "Report that a logical project path changed without a reliable filesystem event.\n\nThe watcher routes the path through its configured change/ignore rules exactly like a native change: matched tasks run under the same ordering, templates, and busy policy. The path need not exist, so deletions and remote logical events are representable. This is not a generic event bus and it does not mutate the filesystem. With `--wait` the scheduled generation is awaited atomically and its terminal observation returned; a no-op (unmatched/ignored) emit stays an explicit no-op.",
                        )
                        .arg(
                            Arg::new("wait")
                                .long("wait")
                                .action(clap::ArgAction::SetTrue)
                                .requires("timeout")
                                .help("Await the scheduled generation to terminal and return its observation."),
                        )
                        .arg(
                            Arg::new("timeout")
                                .long("timeout")
                                .value_name("DURATION")
                                .requires("wait")
                                .value_parser(clap::builder::ValueParser::new(crate::cli::control::parse_duration))
                                .help("Bound for the await: <number> seconds, or <number>ms/s/m (required with --wait)."),
                        )
                        .arg(
                            Arg::new("path")
                                .value_name("PATH")
                                .num_args(1)
                                .required(true)
                                .value_parser(clap::builder::ValueParser::new(|value: &str| {
                                    if value.trim().is_empty() {
                                        Err("path cannot be empty".to_string())
                                    } else {
                                        Ok(value.to_string())
                                    }
                                }))
                                .help("Project path that changed (relative or absolute)."),
                        ),
                )
                .subcommand(
                    Command::new("await")
                        .about("Atomically await a generation to terminal and return one consistent observation.")
                        .version(env!("CARGO_PKG_VERSION"))
                        .long_about(
                            "Atomically await a generation to terminal and return one consistent observation.\n\nThe server observes the current sequence and registers the waiter under one lock, so no transition is lost between snapshot read and waiter registration; waiters never block watcher scheduling. `--after N` returns the next terminal generation after N (or immediately when one exists); `--generation N` returns when the exact generation N reaches terminal. Superseded generations return immediately with reason `superseded`. Timeouts bound the wait, perform no cancellation, and report the latest snapshot.",
                        )
                        .arg(
                            Arg::new("after")
                                .long("after")
                                .value_name("GENERATION")
                                .conflicts_with("generation")
                                .requires("timeout")
                                .value_parser(clap::value_parser!(u64))
                                .help("Await the next terminal generation strictly after this one."),
                        )
                        .arg(
                            Arg::new("generation")
                                .long("generation")
                                .value_name("GENERATION")
                                .conflicts_with("after")
                                .requires("timeout")
                                .value_parser(clap::value_parser!(u64))
                                .help("Await this exact generation to terminal."),
                        )
                        .arg(
                            Arg::new("timeout")
                                .long("timeout")
                                .value_name("DURATION")
                                .required(true)
                                .value_parser(clap::builder::ValueParser::new(crate::cli::control::parse_duration))
                                .help("Bound for the await: <number> seconds, or <number>ms/s/m (always required; awaits are never unbounded)."),
                        )
                        .group(
                            clap::ArgGroup::new("mode")
                                .args(["after", "generation"])
                                .required(true)
                                .multiple(false),
                        ),
                )
                .subcommand(
                    Command::new("cancel")
                        .about("Cancel an exact generation on the running watcher.")
                        .version(env!("CARGO_PKG_VERSION"))
                        .long_about(
                            "Cancel an exact generation on the running watcher.\n\nThe server compares generation identity atomically: a stale request never affects a replacement or newer run, and an already-terminal generation is a no-op. Cancellation uses the graceful process-group shutdown and escalates when the child ignores the signal. With `--wait` the exact generation is awaited atomically and its terminal observation is returned.",
                        )
                        .arg(
                            Arg::new("generation")
                                .long("generation")
                                .value_name("GENERATION")
                                .required(true)
                                .value_parser(clap::value_parser!(u64))
                                .help("Exact generation to cancel."),
                        )
                        .arg(
                            Arg::new("wait")
                                .long("wait")
                                .action(clap::ArgAction::SetTrue)
                                .requires("timeout")
                                .help("Await the exact generation to terminal after cancelling."),
                        )
                        .arg(
                            Arg::new("timeout")
                                .long("timeout")
                                .value_name("DURATION")
                                .requires("wait")
                                .value_parser(clap::builder::ValueParser::new(crate::cli::control::parse_duration))
                                .help("Bound for the await: <number> seconds, or <number>ms/s/m (required with --wait)."),
                        ),
                )
                .subcommand(
                    Command::new("output")
                        .about("Retrieve bounded retained task output for a generation.")
                        .version(env!("CARGO_PKG_VERSION"))
                        .long_about(
                            "Retrieve bounded retained task output for a generation.\n\nOutput is captured per task and per stream (stdout/stderr) up to a declared per-stream byte bound, and retained globally across generations up to a declared byte budget with oldest-generation-first eviction. Truncation is always marked. `--full` returns everything still retained — bounded by the declared retention limit. Command output may contain secrets: the control socket permission (0600) is the security boundary, not this tool.",
                        )
                        .arg(
                            Arg::new("generation")
                                .long("generation")
                                .value_name("GENERATION")
                                .required(true)
                                .value_parser(clap::value_parser!(u64))
                                .help("Generation whose retained output to retrieve."),
                        )
                        .arg(
                            Arg::new("task")
                                .long("task")
                                .value_name("TASK")
                                .value_parser(clap::builder::ValueParser::string())
                                .help("Task name to restrict retrieval to (default: all retained tasks)."),
                        )
                        .arg(
                            Arg::new("stdout")
                                .long("stdout")
                                .action(clap::ArgAction::SetTrue)
                                .conflicts_with("stderr")
                                .help("Restrict to stdout only."),
                        )
                        .arg(
                            Arg::new("stderr")
                                .long("stderr")
                                .action(clap::ArgAction::SetTrue)
                                .conflicts_with("stdout")
                                .help("Restrict to stderr only."),
                        )
                        .arg(
                            Arg::new("tail")
                                .long("tail")
                                .value_name("LINES")
                                .conflicts_with_all(["full", "page"])
                                .value_parser(clap::value_parser!(u64))
                                .help("Last N lines per stream (default: 40)."),
                        )
                        .arg(
                            Arg::new("full")
                                .long("full")
                                .action(clap::ArgAction::SetTrue)
                                .conflicts_with_all(["tail", "page"])
                                .help("Legacy: return everything still retained, translated to the first bounded page with a continuation cursor (never above the transport budget)."),
                        )
                        .arg(
                            Arg::new("page")
                                .long("page")
                                .action(clap::ArgAction::SetTrue)
                                .conflicts_with_all(["tail", "full"])
                                .help("Page retained output deterministically below the negotiated transport budget (contract §5)."),
                        )
                        .arg(
                            Arg::new("page-size")
                                .long("page-size")
                                .value_name("BYTES")
                                .requires("page")
                                .value_parser(clap::value_parser!(u64))
                                .help("Serialized page budget in bytes (default: conservative negotiated limit)."),
                        )
                        .arg(
                            Arg::new("cursor")
                                .long("cursor")
                                .value_name("CURSOR")
                                .requires("page")
                                .value_parser(clap::builder::ValueParser::string())
                                .help("Opaque continuation cursor from a previous page response."),
                        ),
                ),
        )
}

const HELP_TEMPLATE: &str = "\
{before-help}Funzzy the watcher.

Alias:
  fzz -> funzzy

{usage-heading} {usage}

Commands:
  init                Create or migrate a '.watch.yaml' file.
  watch [TARGET]      Watch for file changes and run configured tasks.
  list                List configured tasks.
  run TARGET          Run configured tasks once locally, without watcher IPC.
  explain PATH        Show which tasks a path matches or is ignored by.
  exec                Run an ad-hoc command over stdin-supplied paths.
  control             Interact with a running watcher over its control socket.

{all-args}

Environment configs:
  FUNZZY_NON_BLOCK        Same as `--on-busy restart`
  FUNZZY_BAIL             Same as `--fail-fast`
  FUNZZY_COLORED          Output with colors.
  FUNZZY_STDIN_TIMEOUT_MS Timeout in milliseconds waiting for stdin data (default: 2000)
{after-help}
";

#[cfg(test)]
mod tests {
    use super::*;

    pub(super) fn parse(args: &[&str]) -> Result<Arguments, clap::Error> {
        Arguments::try_parse_from(std::iter::once("fzz").chain(args.iter().copied()))
    }

    fn parse_action(args: &[&str]) -> Action {
        parse(args).expect("expected parse to succeed").action
    }

    // -------------------------------------------------------------------
    // Commands
    // -------------------------------------------------------------------

    #[test]
    fn no_arguments_selects_configured_watch() {
        assert_eq!(parse_action(&[]), Action::Watch { target: None });
    }

    #[test]
    fn watch_subcommand_selects_configured_watch() {
        assert_eq!(parse_action(&["watch"]), Action::Watch { target: None });
    }

    #[test]
    fn watch_with_target_carries_target() {
        assert_eq!(
            parse_action(&["watch", "@quick"]),
            Action::Watch {
                target: Some("@quick".to_string()),
            }
        );
    }

    #[test]
    fn watch_sequential_flag_sets_sequential() {
        let args = parse(&["watch", "@quick", "--sequential"]).expect("parse");
        assert!(args.sequential);
        assert_eq!(
            args.action,
            Action::Watch {
                target: Some("@quick".to_string()),
            }
        );
        let args = parse(&["watch", "--sequential"]).expect("parse");
        assert!(args.sequential);
        let args = parse(&["watch", "@quick"]).expect("parse");
        assert!(!args.sequential);
    }

    #[test]
    fn list_subcommand_selects_list() {
        assert_eq!(parse_action(&["list"]), Action::List);
    }

    #[test]
    fn check_subcommand_selects_check() {
        assert_eq!(parse_action(&["check"]), Action::Check);
        let args = parse(&["check"]).expect("check parses");
        assert_eq!(args.action, Action::Check);
    }

    #[test]
    fn run_subcommand_requires_and_carries_target() {
        assert_eq!(
            parse_action(&["run", "@quick"]),
            Action::Run {
                target: "@quick".to_owned()
            }
        );
        assert!(parse(&["run"]).is_err());
        assert!(parse(&["run", "@quick", "src/lib.rs"]).is_err());
    }

    #[test]
    fn run_sequential_flag_sets_sequential() {
        let args = parse(&["run", "@quick", "--sequential"]).expect("parse");
        assert!(args.sequential);
        assert_eq!(
            args.action,
            Action::Run {
                target: "@quick".to_owned()
            }
        );
        let args = parse(&["run", "--sequential", "@quick"]).expect("parse");
        assert!(args.sequential);
        let args = parse(&["run", "@quick"]).expect("parse");
        assert!(!args.sequential);
    }

    #[test]
    fn init_subcommand_selects_init() {
        assert_eq!(parse_action(&["init"]), Action::Init);
    }

    #[test]
    fn init_with_migrate_flag_sets_migrate() {
        let args = parse(&["init", "--migrate"]).expect("parse");
        assert_eq!(args.action, Action::Init);
        assert!(args.migrate);
    }

    #[test]
    fn migrate_without_init_is_unknown() {
        // `--migrate` is scoped to `init`; without it, it is not a valid flag.
        assert!(parse(&["--migrate"]).is_err());
    }

    // -------------------------------------------------------------------
    // control
    // -------------------------------------------------------------------

    #[test]
    fn control_without_subcommand_fails() {
        assert!(parse(&["control"]).is_err());
    }

    #[test]
    fn control_status_selects_status() {
        assert_eq!(
            parse_action(&["control", "status"]),
            Action::Control {
                action: ControlAction::Status,
                socket: None,
                format: OutputFormat::Human,
            }
        );
    }

    #[test]
    fn control_list_selects_list() {
        assert_eq!(
            parse_action(&["control", "list"]),
            Action::Control {
                action: ControlAction::List,
                socket: None,
                format: OutputFormat::Human,
            }
        );
    }

    #[test]
    fn control_capabilities_selects_capabilities() {
        assert_eq!(
            parse_action(&["control", "capabilities"]),
            Action::Control {
                action: ControlAction::Capabilities,
                socket: None,
                format: OutputFormat::Human,
            }
        );
    }

    #[test]
    fn control_config_selects_config() {
        assert_eq!(
            parse_action(&["control", "config"]),
            Action::Control {
                action: ControlAction::Config,
                socket: None,
                format: OutputFormat::Human,
            }
        );
    }

    #[test]
    fn control_run_carries_target() {
        assert_eq!(
            parse_action(&["control", "run", "@agent-final"]),
            Action::Control {
                action: ControlAction::Run {
                    target: "@agent-final".to_string(),
                    wait: false,
                    timeout: None,
                    sequential: false,
                },
                socket: None,
                format: OutputFormat::Human,
            }
        );
    }

    #[test]
    fn control_run_sequential_flag_carries_through() {
        assert_eq!(
            parse_action(&["control", "run", "@agent-final", "--sequential"]),
            Action::Control {
                action: ControlAction::Run {
                    target: "@agent-final".to_string(),
                    wait: false,
                    timeout: None,
                    sequential: true,
                },
                socket: None,
                format: OutputFormat::Human,
            }
        );
        assert_eq!(
            parse_action(&["ctl", "run", "@agent-final", "--sequential"]),
            Action::Control {
                action: ControlAction::Run {
                    target: "@agent-final".to_string(),
                    wait: false,
                    timeout: None,
                    sequential: true,
                },
                socket: None,
                format: OutputFormat::Human,
            }
        );
    }

    #[test]
    fn control_run_without_target_fails() {
        assert!(parse(&["control", "run"]).is_err());
    }

    #[test]
    fn control_socket_flag_carries_path() {
        let args = parse(&["control", "--socket", "/tmp/sock", "status"]).expect("parse");
        match args.action {
            Action::Control {
                action,
                socket,
                format: _,
            } => {
                assert_eq!(action, ControlAction::Status);
                assert_eq!(socket.as_deref(), Some("/tmp/sock"));
            }
            other => panic!("expected Control action, got {:?}", other),
        }
    }

    #[test]
    fn control_status_accepts_global_control_socket() {
        let args = parse(&["--control-socket", "/tmp/global", "control", "status"]).expect("parse");
        assert_eq!(args.control_socket.as_deref(), Some("/tmp/global"));
        assert_eq!(
            args.action,
            Action::Control {
                action: ControlAction::Status,
                socket: None,
                format: OutputFormat::Human,
            }
        );
    }

    #[test]
    fn control_unknown_subcommand_fails() {
        assert!(parse(&["control", "bogus"]).is_err());
        assert!(parse(&["ctl", "bogus"]).is_err());
    }

    #[test]
    fn ctl_alias_parses_identically_to_control() {
        // TASK-0070: `ctl` is a visible alias for canonical `control`;
        // every nested operation must produce the exact same Action.
        let canonical = [
            &["control", "capabilities"][..],
            &["control", "status"][..],
            &["control", "list"][..],
            &["control", "run", "@agent-final"][..],
            &[
                "control",
                "run",
                "@agent-final",
                "--wait",
                "--timeout",
                "30",
            ][..],
            &["control", "emit", "src/main.rs"][..],
            &["control", "emit", "x.txt", "--wait", "--timeout", "2m"][..],
            &["control", "await", "--after", "3", "--timeout", "2s"][..],
            &[
                "control",
                "await",
                "--generation",
                "9",
                "--timeout",
                "500ms",
            ][..],
            &[
                "control",
                "output",
                "--generation",
                "7",
                "--task",
                "my tests",
                "--stderr",
                "--tail",
                "80",
            ][..],
            &["control", "cancel", "--generation", "7"][..],
            &[
                "control",
                "cancel",
                "--generation",
                "7",
                "--wait",
                "--timeout",
                "1s",
            ][..],
        ];
        for args in canonical {
            let mut alias: Vec<&str> = Vec::with_capacity(args.len());
            alias.push("ctl");
            alias.extend_from_slice(&args[1..]);
            assert_eq!(
                parse_action(&alias),
                parse_action(args),
                "ctl alias must equal control for {:?}",
                args
            );
        }
    }

    #[test]
    fn ctl_socket_flag_and_global_socket_carry_through() {
        let args = parse(&["ctl", "--socket", "/tmp/sock", "status"]).expect("parse");
        match args.action {
            Action::Control {
                action,
                socket,
                format: _,
            } => {
                assert_eq!(action, ControlAction::Status);
                assert_eq!(socket.as_deref(), Some("/tmp/sock"));
            }
            other => panic!("expected Control action, got {:?}", other),
        }
        let args = parse(&["--control-socket", "/tmp/global", "ctl", "status"]).expect("parse");
        assert_eq!(args.control_socket.as_deref(), Some("/tmp/global"));
        assert_eq!(
            args.action,
            Action::Control {
                action: ControlAction::Status,
                socket: None,
                format: OutputFormat::Human,
            }
        );
    }

    #[test]
    fn control_emit_carries_path() {
        assert_eq!(
            parse_action(&["control", "emit", "src/main.rs"]),
            Action::Control {
                action: ControlAction::Emit {
                    path: "src/main.rs".to_string(),
                    wait: false,
                    timeout: None,
                },
                socket: None,
                format: OutputFormat::Human,
            }
        );
    }

    #[test]
    fn control_emit_without_path_fails() {
        assert!(parse(&["control", "emit"]).is_err());
    }

    #[test]
    fn control_emit_rejects_empty_path() {
        assert!(parse(&["control", "emit", ""]).is_err());
        assert!(parse(&["control", "emit", "   "]).is_err());
    }

    #[test]
    fn control_await_after_carries_mode_and_timeout() {
        assert_eq!(
            parse_action(&["control", "await", "--after", "3", "--timeout", "2s"]),
            Action::Control {
                action: ControlAction::Await {
                    after: Some(3),
                    generation: None,
                    timeout: Duration::from_secs(2),
                },
                socket: None,
                format: OutputFormat::Human,
            }
        );
    }

    #[test]
    fn control_await_generation_carries_mode_and_timeout() {
        assert_eq!(
            parse_action(&[
                "control",
                "await",
                "--generation",
                "9",
                "--timeout",
                "500ms"
            ]),
            Action::Control {
                action: ControlAction::Await {
                    after: None,
                    generation: Some(9),
                    timeout: Duration::from_millis(500),
                },
                socket: None,
                format: OutputFormat::Human,
            }
        );
    }

    #[test]
    fn control_await_without_timeout_fails() {
        assert!(parse(&["control", "await", "--after", "3"]).is_err());
        assert!(parse(&["control", "await", "--generation", "3"]).is_err());
    }

    #[test]
    fn control_await_both_modes_conflict() {
        assert!(parse(&[
            "control",
            "await",
            "--after",
            "1",
            "--generation",
            "2",
            "--timeout",
            "1s"
        ])
        .is_err());
    }

    #[test]
    fn control_await_without_mode_fails() {
        assert!(parse(&["control", "await", "--timeout", "1s"]).is_err());
    }

    #[test]
    fn control_await_rejects_invalid_duration() {
        assert!(parse(&["control", "await", "--after", "1", "--timeout", "1h"]).is_err());
        assert!(parse(&["control", "await", "--after", "1", "--timeout", "0s"]).is_err());
    }

    #[test]
    fn control_run_wait_requires_timeout() {
        assert!(parse(&["control", "run", "target", "--wait"]).is_err());
        assert!(parse(&["control", "run", "target", "--timeout", "1s"]).is_err());
    }

    #[test]
    fn control_run_wait_carries_timeout() {
        let action = parse_action(&[
            "control",
            "run",
            "@agent-final",
            "--wait",
            "--timeout",
            "30",
        ]);
        assert_eq!(
            action,
            Action::Control {
                action: ControlAction::Run {
                    target: "@agent-final".to_string(),
                    wait: true,
                    timeout: Some(Duration::from_secs(30)),
                    sequential: false,
                },
                socket: None,
                format: OutputFormat::Human,
            }
        );
    }

    #[test]
    fn control_emit_wait_carries_timeout() {
        let action = parse_action(&["control", "emit", "x.txt", "--wait", "--timeout", "2m"]);
        assert_eq!(
            action,
            Action::Control {
                action: ControlAction::Emit {
                    path: "x.txt".to_string(),
                    wait: true,
                    timeout: Some(Duration::from_secs(120)),
                },
                socket: None,
                format: OutputFormat::Human,
            }
        );
    }

    #[test]
    fn control_output_carries_generation_and_filters() {
        let action = parse_action(&[
            "control",
            "output",
            "--generation",
            "7",
            "--task",
            "my tests",
            "--stderr",
            "--tail",
            "80",
        ]);
        assert_eq!(
            action,
            Action::Control {
                action: ControlAction::Output {
                    generation: 7,
                    task: Some("my tests".to_string()),
                    stream: Some("stderr".to_string()),
                    tail: Some(80),
                    full: false,
                    page: false,
                    page_size: None,
                    cursor: None,
                },
                socket: None,
                format: OutputFormat::Human,
            }
        );
    }

    #[test]
    fn control_output_requires_generation_and_rejects_conflicts() {
        assert!(parse(&["control", "output"]).is_err());
        assert!(parse(&[
            "control",
            "output",
            "--generation",
            "1",
            "--stdout",
            "--stderr"
        ])
        .is_err());
        assert!(parse(&[
            "control",
            "output",
            "--generation",
            "1",
            "--tail",
            "5",
            "--full"
        ])
        .is_err());
        // Contract §2: page is structurally exclusive with tail and full, and
        // page-only options require page mode — rejected before the socket
        // call (exit 2), never a combination the server must disambiguate.
        assert!(parse(&[
            "control",
            "output",
            "--generation",
            "1",
            "--page",
            "--tail",
            "5"
        ])
        .is_err());
        assert!(parse(&["control", "output", "--generation", "1", "--page", "--full"]).is_err());
        assert!(
            parse(&[
                "control",
                "output",
                "--generation",
                "1",
                "--page-size",
                "4096"
            ])
            .is_err(),
            "--page-size requires --page"
        );
        assert!(
            parse(&[
                "control",
                "output",
                "--generation",
                "1",
                "--cursor",
                "7|0|0|0"
            ])
            .is_err(),
            "--cursor requires --page"
        );
    }

    #[test]
    fn control_output_page_flags_parse_with_mode() {
        let action = parse_action(&[
            "control",
            "output",
            "--generation",
            "7",
            "--page",
            "--page-size",
            "8192",
        ]);
        assert_eq!(
            action,
            Action::Control {
                action: ControlAction::Output {
                    generation: 7,
                    task: None,
                    stream: None,
                    tail: None,
                    full: false,
                    page: true,
                    page_size: Some(8192),
                    cursor: None,
                },
                socket: None,
                format: OutputFormat::Human,
            }
        );

        let with_cursor = parse_action(&[
            "control",
            "output",
            "--generation",
            "7",
            "--page",
            "--cursor",
            "7|0|0|8",
        ]);
        assert_eq!(
            with_cursor,
            Action::Control {
                action: ControlAction::Output {
                    generation: 7,
                    task: None,
                    stream: None,
                    tail: None,
                    full: false,
                    page: true,
                    page_size: None,
                    cursor: Some("7|0|0|8".to_string()),
                },
                socket: None,
                format: OutputFormat::Human,
            }
        );
    }

    #[test]
    fn control_cancel_carries_generation_and_wait() {
        assert_eq!(
            parse_action(&["control", "cancel", "--generation", "7"]),
            Action::Control {
                action: ControlAction::Cancel {
                    generation: 7,
                    wait: false,
                    timeout: None,
                },
                socket: None,
                format: OutputFormat::Human,
            }
        );
    }

    #[test]
    fn control_cancel_wait_requires_timeout_and_vice_versa() {
        assert!(parse(&["control", "cancel", "--generation", "7", "--wait"]).is_err());
        assert!(parse(&["control", "cancel", "--generation", "7", "--timeout", "1s"]).is_err());
    }

    #[test]
    fn control_cancel_requires_generation() {
        assert!(parse(&["control", "cancel"]).is_err());
    }

    #[test]
    fn exec_captures_trailing_command_argv() {
        let action = parse_action(&["exec", "echo", "hello world"]);
        assert_eq!(
            action,
            Action::Exec {
                command: vec!["echo".to_string(), "hello world".to_string()]
            }
        );
    }

    #[test]
    fn exec_after_double_dash_captures_flag_like_args() {
        let action = parse_action(&["exec", "--", "cargo", "test", "--release"]);
        assert_eq!(
            action,
            Action::Exec {
                command: vec![
                    "cargo".to_string(),
                    "test".to_string(),
                    "--release".to_string(),
                ]
            }
        );
    }

    #[test]
    fn exec_without_command_fails() {
        assert!(parse(&["exec"]).is_err());
    }

    #[test]
    fn exec_double_dash_without_command_fails() {
        // `--` must not swallow a missing command: exec requires argv.
        assert!(parse(&["exec", "--"]).is_err());
    }

    #[test]
    fn exec_allows_flag_like_first_argument() {
        // A child program may itself start with `-`; the boundary is `--`.
        let action = parse_action(&["exec", "--", "--helper", "run"]);
        assert_eq!(
            action,
            Action::Exec {
                command: vec!["--helper".to_string(), "run".to_string()]
            }
        );
    }

    #[test]
    fn unknown_subcommand_fails() {
        // The bare ad-hoc form `fzz '<command>'` is removed in V2; the first
        // positional is now treated as a subcommand name and rejected.
        assert!(parse(&["echo", "hello"]).is_err());
    }

    // -------------------------------------------------------------------
    // Options (global, usable with any subcommand)
    // -------------------------------------------------------------------

    #[test]
    fn short_version_flag_is_handled_by_clap() {
        let err = parse(&["-V"]).unwrap_err();
        assert_eq!(err.kind(), clap::error::ErrorKind::DisplayVersion);
    }

    #[test]
    fn long_version_flag_is_handled_by_clap() {
        let err = parse(&["--version"]).unwrap_err();
        assert_eq!(err.kind(), clap::error::ErrorKind::DisplayVersion);
    }

    #[test]
    fn verbose_short_flag_is_verbose_not_version() {
        let args = parse(&["-v"]).expect("parse");
        assert!(args.verbose);
        assert_eq!(args.action, Action::Watch { target: None });
    }

    #[test]
    fn verbose_long_flag_is_verbose() {
        let args = parse(&["--verbose"]).expect("parse");
        assert!(args.verbose);
    }

    #[test]
    fn config_option_accepts_equals_form() {
        let args = parse(&["--config=/some/path"]).expect("parse");
        assert_eq!(args.config.as_deref(), Some("/some/path"));
    }

    #[test]
    fn global_config_propagates_to_subcommand() {
        let args = parse(&["watch", "-c", "/some/path"]).expect("parse");
        assert_eq!(args.config.as_deref(), Some("/some/path"));
        assert_eq!(args.action, Action::Watch { target: None });
    }

    #[test]
    fn short_config_equals_form_uses_value_without_equals() {
        let args = parse(&["-c=/some/path"]).expect("parse");
        assert_eq!(args.config.as_deref(), Some("/some/path"));
    }

    #[test]
    fn empty_config_value_falls_back_to_default() {
        let args = parse(&["--config="]).expect("parse");
        assert_eq!(args.config, None);
    }

    #[test]
    fn target_flag_is_removed() {
        // V2 removed --target/-t in favor of `watch TARGET` and `list`.
        assert!(parse(&["-t"]).is_err());
        assert!(parse(&["--target"]).is_err());
        assert!(parse(&["--target", "@quick"]).is_err());
    }

    #[test]
    fn on_busy_defaults_to_wait() {
        let args = parse(&[]).expect("parse");
        assert_eq!(args.on_busy, OnBusy::Wait);
    }

    #[test]
    fn on_busy_restart_flag_selects_restart() {
        let args = parse(&["--on-busy", "restart"]).expect("parse");
        assert_eq!(args.on_busy, OnBusy::Restart);
    }

    #[test]
    fn restart_alias_selects_restart() {
        let args = parse(&["--restart"]).expect("parse");
        assert_eq!(args.on_busy, OnBusy::Restart);
    }

    #[test]
    fn on_busy_invalid_value_fails() {
        assert!(parse(&["--on-busy", "bogus"]).is_err());
    }

    #[test]
    fn non_block_flag_is_removed() {
        // V2 removed --non-block in favor of --on-busy restart.
        assert!(parse(&["--non-block"]).is_err());
        assert!(parse(&["-n"]).is_err());
    }

    #[test]
    fn log_truncate_and_log_file_options() {
        let args = parse(&["-T", "-l", "out.log"]).expect("parse");
        assert!(args.log_truncate_on_change);
        assert_eq!(args.log_file.as_deref(), Some("out.log"));
    }

    #[test]
    fn control_socket_option() {
        let args = parse(&["--control-socket", "/tmp/sock"]).expect("parse");
        assert_eq!(args.control_socket.as_deref(), Some("/tmp/sock"));
    }

    #[test]
    fn no_run_on_init_option() {
        let args = parse(&["--no-run-on-init"]).expect("parse");
        assert!(args.no_run_on_init);
    }

    // -------------------------------------------------------------------
    // Unhappy paths
    // -------------------------------------------------------------------

    #[test]
    fn unknown_long_option_fails() {
        assert!(parse(&["--bogus"]).is_err());
    }

    #[test]
    fn unknown_short_option_fails() {
        assert!(parse(&["-z"]).is_err());
    }

    #[test]
    fn missing_value_for_config_option_fails() {
        assert!(parse(&["-c"]).is_err());
    }

    #[test]
    fn missing_value_for_log_file_option_fails() {
        assert!(parse(&["--log-file"]).is_err());
    }
}

#[cfg(test)]
mod format_tests {
    use super::tests::parse;
    use super::Action;
    use crate::cli::{ControlAction, OutputFormat};

    #[test]
    fn control_format_flag_selects_output_format() {
        let args = parse(&["control", "--format", "json", "status"]).expect("parse");
        match args.action {
            Action::Control { format, action, .. } => {
                assert_eq!(format, OutputFormat::Json);
                assert_eq!(action, ControlAction::Status);
            }
            other => panic!("expected Control, got {:?}", other),
        }
        let args = parse(&["ctl", "--format", "toon", "list"]).expect("parse");
        match args.action {
            Action::Control { format, .. } => assert_eq!(format, OutputFormat::Toon),
            other => panic!("expected Control, got {:?}", other),
        }
        let args = parse(&["control", "status"]).expect("parse");
        match args.action {
            Action::Control { format, .. } => assert_eq!(format, OutputFormat::Human),
            other => panic!("expected Control, got {:?}", other),
        }
    }

    #[test]
    fn control_format_rejects_unknown_values() {
        assert!(parse(&["control", "--format", "xml", "status"]).is_err());
    }

    #[test]
    fn control_format_works_through_ctl_alias_and_after_socket() {
        let args =
            parse(&["ctl", "--socket", "/tmp/s", "--format", "json", "status"]).expect("parse");
        match args.action {
            Action::Control { format, socket, .. } => {
                assert_eq!(format, OutputFormat::Json);
                assert_eq!(socket.as_deref(), Some("/tmp/s"));
            }
            other => panic!("expected Control, got {:?}", other),
        }
    }
}

#[cfg(test)]
mod config_command_tests {
    use super::tests::parse;
    use super::Action;
    use crate::cli::OutputFormat;

    #[test]
    fn config_schema_parses_with_optional_section() {
        let args = parse(&["config", "schema"]).expect("parse");
        assert_eq!(
            args.action,
            Action::Config {
                schema_section: Some(None),
                example_profile: None,
                format: OutputFormat::Human,
            }
        );
        let args = parse(&["config", "schema", "--section", "parallel"]).expect("parse");
        match args.action {
            Action::Config {
                schema_section: Some(Some(section)),
                example_profile: None,
                format: OutputFormat::Human,
            } => assert_eq!(section, "parallel"),
            other => panic!("expected config schema, got {:?}", other),
        }
    }

    #[test]
    fn config_example_parses_profile() {
        let args = parse(&["config", "example", "agent"]).expect("parse");
        match args.action {
            Action::Config {
                schema_section: None,
                example_profile: Some(profile),
                format: OutputFormat::Human,
            } => assert_eq!(profile, "agent"),
            other => panic!("expected config example, got {:?}", other),
        }
    }

    #[test]
    fn config_rejects_unknown_section_and_profile() {
        assert!(parse(&["config", "schema", "--section", "bogus"]).is_err());
        assert!(parse(&["config", "example", "bogus"]).is_err());
        assert!(parse(&["config", "example"]).is_err());
    }
}

#[cfg(test)]
mod completions_tests {
    use super::tests::parse;
    use super::Action;

    #[test]
    fn completions_parses_shell_and_rejects_unknown() {
        let args = parse(&["completions", "zsh"]).expect("parse");
        assert_eq!(
            args.action,
            Action::Completions {
                shell: "zsh".to_owned()
            }
        );
        assert!(parse(&["completions"]).is_err());
        assert!(parse(&["completions", "tcsh"]).is_err());
    }
}
