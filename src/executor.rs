//! Shared task execution core (TASK-0026).
//!
//! Owns process spawn, fail-fast, and per-command outcome collection at
//! concurrency one. Blocking execution drives this synchronously today; the
//! restart path (`workers::ActiveRun`) will delegate to the same spawn/wait
//! semantics asynchronously in the worker-side unification, so both busy-run
//! policies share one command loop instead of duplicating it.
//!
//! Design intent (full TASK-0026): the executor receives a process runner,
//! clock, concurrency limit, and event sink explicitly; blocking and restart
//! strategies only decide busy-run policy and submit/cancel plans.

use crate::cmd::{self, LoggedChild};
use crate::rules::CommandLine;
use crate::stdout;
use nix::sys::signal::{self, Signal};
use nix::unistd::Pid;
use std::collections::VecDeque;
use std::time::{Duration, Instant};

/// Runs commands synchronously in declared order with fail-fast semantics,
/// collecting one result per command that actually ran.
///
/// Concurrency is one: this is the exact behavior a `RunPlan` produces when
/// flattened and executed sequentially, and it is the shared core both
/// busy-run policies must agree on (acceptance: same plan, equivalent
/// outcomes in wait and restart modes at concurrency one).
pub fn run_commands(commands: Vec<CommandLine>, fail_fast: bool) -> Vec<Result<(), String>> {
    let mut results = vec![];
    for command in commands {
        let result = run_one(&command);
        let stop = fail_fast && result.is_err();
        results.push(result);
        if stop {
            break;
        }
    }
    results
}

fn run_one(command: &CommandLine) -> Result<(), String> {
    match command {
        CommandLine::Shell(command) => cmd::execute(command),
        CommandLine::Argv(argv) => cmd::execute_argv(argv),
    }
}

/// One step of an async (cancel-aware) run.
pub enum Step {
    /// A child is executing; the caller may poll for replacements.
    Running,
    /// Every command finished (or fail-fast stopped the run).
    Finished,
}

/// A run being executed by the shared engine. Owns process spawn
/// (`cmd::spawn`/`spawn_argv`), polling/waiting, fail-fast, cancellation
/// (SIGTERM + reap), and per-command outcome collection at concurrency one.
///
/// The restart busy-run policy drives this with `advance`/`cancel` so it can
/// replace the newest generation; the blocking policy uses the synchronous
/// `run_commands` entry point. Both share one engine instead of duplicating
/// the command loop.
pub struct Run {
    commands: VecDeque<CommandLine>,
    results: Vec<Result<(), String>>,
    started: Instant,
    child: Option<LoggedChild>,
    current_task: Option<String>,
    fail_fast: bool,
}

impl Run {
    pub fn new(commands: Vec<CommandLine>, fail_fast: bool) -> Self {
        Run {
            commands: commands.into(),
            results: vec![],
            started: Instant::now(),
            child: None,
            current_task: None,
            fail_fast,
        }
    }

    /// Elapsed wall time since the run started.
    pub fn elapsed(&self) -> Duration {
        self.started.elapsed()
    }

    /// Display form of the command currently executing, if any.
    pub fn current_task(&self) -> Option<&str> {
        self.current_task.as_deref()
    }

    /// Consumes the run, returning the collected per-command results.
    pub fn into_results(self) -> Vec<Result<(), String>> {
        self.results
    }

    /// Advance this run by one step: spawn the next command or poll the active
    /// child. Returns `Running` whenever a child is executing, so the restart
    /// policy can check for a superseding generation between polls.
    pub fn advance(&mut self) -> Step {
        loop {
            if self.child.is_none() {
                let Some(task) = self.commands.pop_front() else {
                    return Step::Finished;
                };
                let display = task.display();
                self.current_task = Some(display.clone());
                let spawn_result = match task {
                    CommandLine::Shell(command) => cmd::spawn(&command),
                    CommandLine::Argv(argv) => cmd::spawn_argv(&argv),
                };
                match spawn_result {
                    Ok(child) => {
                        self.child = Some(child);
                        return Step::Running;
                    }
                    Err(err) => {
                        let failure = format!("Command {} failed to start: {}", display, err);
                        stdout::error(&failure);
                        self.results.push(Err(failure));
                        if self.fail_fast {
                            return Step::Finished;
                        }
                        continue;
                    }
                }
            }

            let task = self.current_task.clone().unwrap_or_default();
            match self.child.as_mut().expect("child is running").try_wait() {
                Ok(None) => return Step::Running,
                Ok(Some(status)) => {
                    self.child = None;
                    self.current_task = None;
                    if status.success() {
                        self.results.push(Ok(()));
                    } else {
                        self.results
                            .push(Err(format!("Command {} has failed with {}", task, status)));
                        if self.fail_fast {
                            return Step::Finished;
                        }
                    }
                }
                Err(err) => {
                    self.child = None;
                    self.current_task = None;
                    self.results
                        .push(Err(format!("Command {} has errored with {}", task, err)));
                    if self.fail_fast {
                        return Step::Finished;
                    }
                }
            }
        }
    }

    /// Gracefully terminate the active child, if any, and discard remaining
    /// commands. The run never finishes normally after this.
    pub fn cancel(&mut self, verbose: bool) {
        if let Some(child) = self.child.as_mut() {
            let task = self.current_task.clone().unwrap_or_default();
            stdout::verbose(&format!("---- cancelling: {:?} ----", task), verbose);

            if let Err(err) = signal::kill(
                Pid::from_raw(child.id() as i32),
                // Sends a SIGTERM signal to the process
                // and allows it to exit gracefully.
                Signal::SIGTERM,
            ) {
                stdout::error(&format!("failed to terminate task {:?}: {:?}", task, err));
            }

            if let Ok(status) = child.wait() {
                stdout::verbose(
                    &format!("---- finished: {:?} status: {} ----", task, status),
                    verbose,
                );
            } else {
                stdout::error(&format!(
                    "failed to wait for the task to finish: {:?}",
                    task
                ));
            }
        }
        self.child = None;
        self.commands.clear();
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn shell(cmd: &str) -> CommandLine {
        CommandLine::Shell(cmd.to_owned())
    }

    #[test]
    fn runs_all_commands_when_all_pass() {
        let results = run_commands(vec![shell("true"), shell("true")], true);
        assert_eq!(results.len(), 2);
        assert!(results.iter().all(|r| r.is_ok()));
    }

    #[test]
    fn fail_fast_stops_at_first_failure() {
        let results = run_commands(vec![shell("false"), shell("true")], true);
        assert_eq!(results.len(), 1, "fail-fast must not run later commands");
        assert!(results[0].is_err());
    }

    #[test]
    fn without_fail_fast_later_commands_still_run_after_a_failure() {
        let results = run_commands(vec![shell("false"), shell("true")], false);
        assert_eq!(results.len(), 2, "non-fail-fast must run every command");
        assert!(results[0].is_err());
        assert!(results[1].is_ok());
    }

    #[test]
    fn empty_command_list_produces_no_results() {
        assert!(run_commands(vec![], true).is_empty());
        assert!(run_commands(vec![], false).is_empty());
    }

    #[test]
    fn argv_commands_run_directly_without_shell() {
        // Argv preservation: a single argument with spaces stays one arg.
        // `sh -c 'echo $#' probe 'a b'` prints 2 only if 'a b' is one arg.
        let argv = CommandLine::Argv(vec![
            "sh".to_owned(),
            "-c".to_owned(),
            "test \"$#\" = 2".to_owned(),
            "probe".to_owned(),
            "a b".to_owned(),
            "c".to_owned(),
        ]);
        let results = run_commands(vec![argv], true);
        assert_eq!(results.len(), 1);
        assert!(results[0].is_ok(), "argv boundary preserved: {:?}", results);
    }

    #[test]
    fn shell_and_argv_dispatch_through_one_runner() {
        let results = run_commands(
            vec![shell("true"), CommandLine::Argv(vec!["true".to_owned()])],
            true,
        );
        assert_eq!(results.len(), 2);
        assert!(results.iter().all(|r| r.is_ok()));
    }
}
