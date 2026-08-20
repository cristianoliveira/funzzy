use std::io::Write;

#[cfg(not(test))]
use crate::environment;
use crate::logging;

// ANSI color codes for terminal output
pub const GREEN: &str = "\x1b[32m";
pub const RED: &str = "\x1b[31m";
pub const BLUE: &str = "\x1b[34m";
pub const RESET: &str = "\x1b[0m";

#[cfg(not(test))]
pub fn is_colored() -> bool {
    environment::is_enabled("FUNZZY_COLORED")
}

#[cfg(test)]
pub fn is_colored() -> bool {
    false
}

pub fn info(msg: &str) {
    let message = if is_colored() {
        format!("{}Funzzy{}: {}", BLUE, RESET, msg)
    } else {
        format!("Funzzy: {}", msg)
    };

    println!("{}", message);
    logging::log_line(&message);
}

pub fn error(msg: &str) {
    let message = if is_colored() {
        format!("{}Funzzy error{}: {}", RED, RESET, msg)
    } else {
        format!("Funzzy error: {}", msg)
    };

    println!("{}", message);
    logging::log_line(&message);
}

pub fn warn(msg: &str) {
    let message = format!("Funzzy warning: {}", msg);
    println!("{}", message);
    logging::log_line(&message);
}

pub fn show_and_exit(text: &str) -> ! {
    println!("{}", text);
    logging::log_line(text);
    std::process::exit(0)
}

pub fn failure(text: &str, err: String) -> ! {
    let header = if is_colored() {
        format!("{}Error{}: {}", RED, RESET, text)
    } else {
        format!("Error: {}", text)
    };

    println!("{}", header);
    logging::log_line(&header);

    println!("{}", err);
    logging::log_line(&err);
    std::process::exit(1)
}

/// Operational/config failure for commands whose contract requires stderr.
/// Kept separate from legacy `failure` so TASK-0098 can correct migration's
/// channel without silently changing every existing CLI surface.
pub fn failure_to_stderr(text: &str, err: String) -> ! {
    let header = if is_colored() {
        format!("{}Error{}: {}", RED, RESET, text)
    } else {
        format!("Error: {}", text)
    };

    eprintln!("{}", header);
    logging::log_line(&header);

    eprintln!("{}", err);
    logging::log_line(&err);
    std::process::exit(1)
}

#[cfg(not(feature = "test-integration"))]
/// Print the time elapsed in seconds in the format "Finished in 0.1234s"
pub fn print_time_elapsed(elapsed: std::time::Duration) {
    let message = format!("Duration: {:.4}s", elapsed.as_secs_f32());
    print!("{}", message);
    let res = std::io::stdout().flush();

    logging::log_plain(&message);

    match res {
        Ok(_) => (),
        Err(e) => {
            warn("Failed to flush stdout, but the program will continue.");
            warn(&format!("Reason: {:?}", e));
        }
    };
}

#[cfg(feature = "test-integration")]
// NOTE: This is for testing purposes only
/// Print mocked time elapsed always as: "Finished in 0.0s"
pub fn print_time_elapsed(_elapsed_param: std::time::Duration) -> () {
    let elapsed = std::time::Duration::from_secs(0);
    let message = format!("Duration: {:.4}s", elapsed.as_secs_f32());

    print!("{}", message);
    std::io::stdout().flush().expect("Failed to flush stdout");
    logging::log_plain(&message);
}

pub fn present_results(
    results: Vec<Result<(), String>>,
    time_elapsed: std::time::Duration,
    outcome: Option<&crate::plan::RunOutcome>,
) {
    let errors: Vec<Result<(), String>> = results.iter().filter(|&r| r.is_err()).cloned().collect();
    let completed = results.iter().filter(|&r| r.is_ok()).count();
    let header = "Funzzy results ----------------------------";
    println!("{}", header);
    logging::log_line(header);

    // TASK-0028: runs with parallel groups render one line per task with its
    // group identity, so the summary identifies group and every task. Ordering
    // inside a named parallel group is explicitly unspecified; lines are keyed
    // by task identity. Serial runs keep today's summary exactly (contract §7).
    let has_groups = outcome
        .map(|outcome| outcome.tasks.iter().any(|(_, group, _)| group.is_some()))
        .unwrap_or(false);
    if has_groups {
        if let Some(outcome) = outcome {
            for (name, group, task_outcome) in &outcome.tasks {
                let identity = match group {
                    Some(group) => format!("[{}] {}", group, name),
                    None => name.clone(),
                };
                let status = match task_outcome {
                    crate::plan::TaskOutcome::Passed => "passed",
                    crate::plan::TaskOutcome::Failed { .. } => "failed",
                    crate::plan::TaskOutcome::Cancelled => "cancelled",
                    crate::plan::TaskOutcome::Skipped => "skipped",
                };
                let message = format!("- {}: {}", identity, status);
                println!("{}", message);
                logging::log_line(&message);
            }
        }
    }

    if !errors.is_empty() {
        if is_colored() {
            print!("{}", RED);
            logging::log_plain(RED);
        }

        errors.iter().for_each(|err| {
            let message = format!("- {}", err.as_ref().unwrap_err());
            println!("{}", message);
            logging::log_line(&message);
        });

        if is_colored() {
            let message = format!("Failure{}; ", RESET);
            print!("{}", message);
            logging::log_plain(&message);
        } else {
            let message = "Failure; ";
            print!("{}", message);
            logging::log_plain(message);
        }
    } else {
        if is_colored() {
            let message = format!("{}Success{}; ", GREEN, RESET);
            print!("{}", message);
            logging::log_plain(&message);
        } else {
            let message = "Success; ";
            print!("{}", message);
            logging::log_plain(message);
        }
    }

    let summary = format!("Completed: {:?}; Failed: {:?}; ", completed, errors.len());
    print!("{}", summary);
    logging::log_plain(&summary);
    print_time_elapsed(time_elapsed);
}

pub fn clear_screen() {
    // See https://archive.ph/d3Z3O
    print!("\n{}[2J", 27 as char);
}
