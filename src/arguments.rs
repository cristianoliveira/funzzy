//! Clap-backed CLI argument parsing.
//!
//! Owns parser details and exposes semantic action/option types to
//! application code. V2 structure (TASK-0015): real Clap subcommands
//! (`init`, `watch`, `exec`); `fzz` with no subcommand is configured watch.
//! `-V`/`--version` is Clap's built-in version flag (stdout, exit 0);
//! `-v`/`--verbose` is the verbose flag; parse errors use Clap's native
//! handling (stderr, exit 2).

use clap::{Arg, ArgAction, Command};

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
    /// `fzz explain PATH`: print which tasks a path matches or is ignored by.
    Explain { path: String },
    /// `fzz init [--migrate]`: create or migrate the default config file.
    Init,
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
    pub control_socket: Option<String>,
    pub migrate: bool,
    pub on_busy: OnBusy,
    pub no_run_on_init: bool,
    pub fail_fast: bool,
    pub verbose: bool,
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
        let control_socket = matches.get_one::<String>("control_socket").cloned();

        let (action, migrate) = match matches.subcommand() {
            None => (Action::Watch { target: None }, false),
            Some(("watch", sub)) => {
                let target = sub.get_one::<String>("target").cloned();
                (Action::Watch { target }, false)
            }
            Some(("list", _)) => (Action::List, false),
            Some(("explain", sub)) => {
                let path = sub
                    .get_one::<String>("path")
                    .cloned()
                    .expect("path is required by clap");
                (Action::Explain { path }, false)
            }
            Some(("init", sub)) => (Action::Init, sub.get_flag("migrate")),
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
            verbose: matches.get_flag("verbose"),
        })
    }

    /// Rendered help text for the configured command (used by app fallbacks).
    pub fn help_text() -> String {
        command().render_help().to_string()
    }
}

fn command() -> Command {
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
  explain PATH        Show which tasks a path matches or is ignored by.
  exec                Run an ad-hoc command over stdin-supplied paths.

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

    fn parse(args: &[&str]) -> Result<Arguments, clap::Error> {
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
    fn list_subcommand_selects_list() {
        assert_eq!(parse_action(&["list"]), Action::List);
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
