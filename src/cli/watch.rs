use crate::cli::Command;
use crate::cmd;
use crate::config;
use crate::errors::FzzError;
use crate::rules;
use crate::stdout;
use crate::template;
use crate::watcher;
use crate::watches::Watches;

pub const DEFAULT_FILENAME: &str = ".watch.yaml";

/// # `WatchCommand`
///
/// Starts watcher to listen the change events configured
/// in watch.yaml
///
pub struct WatchCommand {
    watches: Watches,
    verbose: bool,
    fail_fast: bool,
    run_on_init: bool,
}

impl WatchCommand {
    pub fn new(watches: Watches, verbose: bool, fail_fast: bool, run_on_init: bool) -> Self {
        stdout::verbose(&format!("watches {:?}", watches), verbose);

        WatchCommand {
            watches,
            verbose,
            fail_fast,
            run_on_init,
        }
    }
}

impl Command for WatchCommand {
    fn execute(&self) -> Result<(), FzzError> {
        stdout::verbose("Verbose mode enabled.", self.verbose);

        let current_dir = self.watches.root();

        let list_of_watched_paths = self.watches.paths_to_watch().unwrap_or_default();

        match watcher::events(
            list_of_watched_paths,
            || {
                if self.run_on_init {
                    if let Some(rules) = self.watches.run_on_init() {
                        let time_execution_started = std::time::Instant::now();

                        stdout::info("Running on init commands.");

                        let expanded = template::template(
                            rules::commands(rules),
                            template::TemplateOptions {
                                filepath: None,
                                current_dir: format!("{}", current_dir.display()),
                            },
                        );
                        for variable in &expanded.unknown_variables {
                            stdout::warn(&format!("Unknown template variable '{}'.", variable));
                        }
                        let tasks = expanded.commands;
                        let mut results: Vec<Result<(), String>> = vec![];
                        for task in tasks {
                            let result = cmd::execute(&task);

                            if self.fail_fast && result.is_err() {
                                results.push(result);
                                break;
                            }

                            results.push(result);
                        }

                        let time_elapsed = time_execution_started.elapsed();
                        stdout::present_results(results, time_elapsed);
                    } else {
                        stdout::info("Watching...");
                    }
                } else {
                    stdout::info("Watching...");
                }
            },
            |file_changed| {
                let time_execution_started = std::time::Instant::now();
                if let Some(rules) = self.watches.watch(file_changed) {
                    stdout::clear_screen();

                    stdout::verbose(
                        &format!("Triggered by change in: {}", file_changed),
                        self.verbose,
                    );

                    let rules_as_yaml = config::format_rules(&rules);
                    stdout::verbose(&format!("Rules: {:?}", rules), self.verbose);
                    stdout::verbose(
                        &format!("Formatted rules:\n{}", rules_as_yaml),
                        self.verbose,
                    );

                    let expanded = template::template(
                        rules::commands(rules),
                        template::TemplateOptions {
                            filepath: Some(file_changed.to_string()),
                            current_dir: format!("{}", current_dir.display()),
                        },
                    );
                    for variable in &expanded.unknown_variables {
                        stdout::warn(&format!("Unknown template variable '{}'.", variable));
                    }
                    let tasks = expanded.commands;
                    let mut results: Vec<Result<(), String>> = vec![];
                    for task in tasks {
                        let result = cmd::execute(&task);

                        if self.fail_fast && result.is_err() {
                            results.push(result);
                            break;
                        }

                        results.push(result);
                    }

                    let elapsed = time_execution_started.elapsed();
                    stdout::present_results(results, elapsed);
                }
            },
            self.verbose,
        ) {
            Ok(_) => Ok(()),
            Err(err) => Err(FzzError::GenericError(err)),
        }
    }
}
