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

use crate::cmd;
use crate::rules::CommandLine;

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
