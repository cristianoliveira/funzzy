//! Clap-backed CLI argument parsing.
//!
//! Owns parser details and exposes semantic action/option types to
//! application code. V2 conventions (TASK-0015): `-V`/`--version` is Clap's
//! built-in version flag (stdout, exit 0); `-v`/`--verbose` is the verbose
//! flag; parse errors use Clap's native handling (stderr, exit 2).

use clap::{Arg, ArgAction, Command};

/// Semantic application action selected from the positional command words.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Action {
    /// `init` — create or migrate the default config file.
    Init,
    /// No command (or the inert `watch` keyword alone): configured watch.
    WatchConfig,
    /// An arbitrary command run against stdin-provided paths.
    WatchCommand(String),
    /// Words that do not form a supported command shape.
    Unexpected(Vec<String>),
}

/// Parser-owned, semantic application arguments.
#[derive(Debug, Clone)]
pub struct Arguments {
    pub action: Action,
    pub config: Option<String>,
    /// Task-name filter; `Some("")` means list-targets mode.
    pub target: Option<String>,
    pub log_truncate_on_change: bool,
    pub log_file: Option<String>,
    pub control_socket: Option<String>,
    pub migrate: bool,
    pub non_block: bool,
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
        let target = matches.get_one::<String>("target").cloned();
        let log_file = matches.get_one::<String>("log_file").cloned();
        let control_socket = matches.get_one::<String>("control_socket").cloned();

        let words: Vec<String> = matches
            .get_many::<String>("command")
            .map(|values| values.cloned().collect())
            .unwrap_or_default();

        let action = match words.as_slice() {
            [] => Action::WatchConfig,
            [word] if word == "init" => Action::Init,
            [word] if word == "watch" => Action::WatchConfig,
            [word, command] if word == "watch" => Action::WatchCommand(command.clone()),
            [command] => Action::WatchCommand(command.clone()),
            _ => Action::Unexpected(words.clone()),
        };

        Ok(Arguments {
            action,
            config,
            target,
            log_truncate_on_change: matches.get_flag("log_truncate_on_change"),
            log_file,
            control_socket,
            migrate: matches.get_flag("migrate"),
            non_block: matches.get_flag("non_block"),
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
        .help_template(HELP_TEMPLATE)
        .arg(
            Arg::new("config")
                .short('c')
                .long("config")
                .value_name("cfgfile")
                .value_parser(clap::builder::ValueParser::string())
                .help("Use given config file."),
        )
        .arg(
            Arg::new("target")
                .short('t')
                .long("target")
                .value_name("name")
                .num_args(0..=1)
                .default_missing_value("")
                .value_parser(clap::builder::ValueParser::string())
                .help("Execute only the given task target (if empty list availables)."),
        )
        .arg(
            Arg::new("non_block")
                .short('n')
                .long("non-block")
                .action(ArgAction::SetTrue)
                .help("Execute tasks and cancel them if a new event is received."),
        )
        .arg(
            Arg::new("fail_fast")
                .short('b')
                .long("fail-fast")
                .action(ArgAction::SetTrue)
                .help("Bail current execution if a task fails (exit code != 0)."),
        )
        .arg(
            Arg::new("log_truncate_on_change")
                .short('T')
                .long("log-truncate-on-change")
                .action(ArgAction::SetTrue)
                .help("Truncate the log file when the config reloads (requires --log-file)."),
        )
        .arg(
            Arg::new("log_file")
                .short('l')
                .long("log-file")
                .value_name("file")
                .value_parser(clap::builder::ValueParser::string())
                .help("Write all output to the specified log file in addition to the console."),
        )
        .arg(
            Arg::new("control_socket")
                .long("control-socket")
                .value_name("path")
                .value_parser(clap::builder::ValueParser::string())
                .help("Expose watcher status over a Unix socket (implies --non-block)."),
        )
        .arg(
            Arg::new("migrate")
                .long("migrate")
                .action(ArgAction::SetTrue)
                .help("Migrate legacy root task list to current format."),
        )
        .arg(
            Arg::new("no_run_on_init")
                .long("no-run-on-init")
                .action(ArgAction::SetTrue)
                .help("Do not run tasks on initialization."),
        )
        .arg(
            Arg::new("verbose")
                .short('v')
                .long("verbose")
                .action(ArgAction::SetTrue)
                .help("Use verbose output."),
        )
        .arg(
            Arg::new("command")
                .value_name("command")
                .num_args(0..=2)
                .value_parser(clap::builder::ValueParser::string())
                .help("Run an arbitrary command for current folder."),
        )
}

const HELP_TEMPLATE: &str = "\
{before-help}Funzzy the watcher.

Alias:
  fzz -> funzzy

{usage-heading} {usage}

Commands:
  init                Create or migrate a '.watch.yaml' file.
  watch               Watch for file changes and execute a command.

{all-args}

Environment configs:
  FUNZZY_NON_BLOCK        Same as `--non-block`
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
        assert_eq!(parse_action(&[]), Action::WatchConfig);
    }

    #[test]
    fn init_selects_init() {
        assert_eq!(parse_action(&["init"]), Action::Init);
    }

    #[test]
    fn init_with_migrate_flag_selects_init_with_migration() {
        let args = parse(&["init", "--migrate"]).expect("parse");
        assert_eq!(args.action, Action::Init);
        assert!(args.migrate);
    }

    #[test]
    fn migrate_without_init_falls_through_to_configured_watch() {
        let args = parse(&["--migrate"]).expect("parse");
        assert_eq!(args.action, Action::WatchConfig);
        assert!(args.migrate);
    }

    #[test]
    fn watch_keyword_alone_is_configured_watch() {
        assert_eq!(parse_action(&["watch"]), Action::WatchConfig);
    }

    #[test]
    fn watch_keyword_with_command_selects_command() {
        assert_eq!(
            parse_action(&["watch", "echo {{filepath}}"]),
            Action::WatchCommand("echo {{filepath}}".to_string())
        );
    }

    #[test]
    fn bare_command_selects_command() {
        assert_eq!(
            parse_action(&["echo {{filepath}}"]),
            Action::WatchCommand("echo {{filepath}}".to_string())
        );
    }

    #[test]
    fn init_with_extra_words_is_unexpected() {
        assert_eq!(
            parse_action(&["init", "extra"]),
            Action::Unexpected(vec!["init".to_string(), "extra".to_string()])
        );
    }

    #[test]
    fn two_arbitrary_words_are_unexpected() {
        assert_eq!(
            parse_action(&["a", "b"]),
            Action::Unexpected(vec!["a".to_string(), "b".to_string()])
        );
    }

    // -------------------------------------------------------------------
    // Options
    // -------------------------------------------------------------------

    #[test]
    fn short_version_flag_is_handled_by_clap() {
        // `-V`/`--version` is Clap's built-in version flag: parsing returns a
        // DisplayVersion error that the process entrypoint renders to stdout
        // and exit 0. It is not an `Action` variant.
        let err = parse(&["-V"]).unwrap_err();
        assert_eq!(err.kind(), clap::error::ErrorKind::DisplayVersion);
    }

    #[test]
    fn long_version_flag_is_handled_by_clap() {
        let err = parse(&["--version"]).unwrap_err();
        assert_eq!(err.kind(), clap::error::ErrorKind::DisplayVersion);
    }

    #[test]
    fn version_wins_over_command_regardless_of_position() {
        for args in [["init", "-V"], ["-V", "init"]] {
            let err = parse(&args).unwrap_err();
            assert_eq!(err.kind(), clap::error::ErrorKind::DisplayVersion);
        }
    }

    #[test]
    fn verbose_short_flag_is_verbose_not_version() {
        // `-v` is verbose; it never prints the version.
        let args = parse(&["-v"]).expect("parse");
        assert!(args.verbose);
        assert_eq!(args.action, Action::WatchConfig);
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
    fn short_config_equals_form_uses_value_without_equals() {
        // Clap treats `-c=<path>` as value `<path>` (Docopt kept the leading
        // `=`). The migration adopts clap semantics.
        let args = parse(&["-c=/some/path"]).expect("parse");
        assert_eq!(args.config.as_deref(), Some("/some/path"));
    }

    #[test]
    fn empty_config_value_falls_back_to_default() {
        let args = parse(&["--config="]).expect("parse");
        assert_eq!(args.config, None);
    }

    #[test]
    fn target_without_value_is_empty_list_mode() {
        let args = parse(&["-t"]).expect("parse");
        assert_eq!(args.target.as_deref(), Some(""));
    }

    #[test]
    fn target_with_explicit_empty_value_is_list_mode() {
        let args = parse(&["--target="]).expect("parse");
        assert_eq!(args.target.as_deref(), Some(""));
    }

    #[test]
    fn target_with_value_keeps_value() {
        let args = parse(&["-t", "run my build"]).expect("parse");
        assert_eq!(args.target.as_deref(), Some("run my build"));
    }

    #[test]
    fn combined_short_flags_are_accepted() {
        let args = parse(&["-nb"]).expect("parse");
        assert!(args.non_block);
        assert!(args.fail_fast);
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

    #[test]
    fn three_positional_words_fail() {
        assert!(parse(&["a", "b", "c"]).is_err());
    }
}
