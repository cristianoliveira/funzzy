use std::io::Write;

use crate::environment;

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
    if is_colored() {
        println!("{}Funzzy{}: {}", BLUE, RESET, msg);
    } else {
        println!("Funzzy: {}", msg);
    }
}

pub fn error(msg: &str) {
    if is_colored() {
        println!("{}Funzzy error{}: {}", RED, RESET, msg);
    } else {
        println!("Funzzy error: {}", msg);
    }
}

pub fn warn(msg: &str) {
    println!("Funzzy warning: {}", msg);
}

pub fn show_and_exit(text: &str) -> ! {
    println!("{}", text);
    std::process::exit(0)
}

pub fn failure(text: &str, err: String) -> ! {
    if is_colored() {
        println!("{}Error{}: {}", RED, RESET, text);
    } else {
        println!("Error: {}", text);
    }
    println!("{}", err);
    std::process::exit(1)
}

pub fn verbose(msg: &str, verbose: bool) {
    if !verbose {
        return;
    }

    println!("-----------------------------");
    println!("Funzzy verbose: {} ", msg);
    println!("-----------------------------");
}

#[cfg(not(feature = "test-integration"))]
/// Print the time elapsed in seconds in the format "Finished in 0.1234s"
pub fn print_time_elapsed(elapsed: std::time::Duration) -> () {
    print!("Durantion: {:.4}s", elapsed.as_secs_f32());
    let res = std::io::stdout().flush();

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
pub fn print_time_elapsed(_: std::time::Duration) -> () {
    let elapsed = std::time::Duration::from_secs(0);
    print!("Durantion: {:.4}s", elapsed.as_secs_f32());
    std::io::stdout().flush().expect("Failed to flush stdout");
}

pub fn present_results(results: Vec<Result<(), String>>, time_elapsed: std::time::Duration) {
    let errors: Vec<Result<(), String>> = results.iter().cloned().filter(|r| r.is_err()).collect();
    let completed = results.iter().cloned().filter(|r| r.is_ok()).count();
    println!("Funzzy results ----------------------------");
    if !errors.is_empty() {
        if is_colored() {
            print!("{}", RED);
        }

        errors.iter().for_each(|err| {
            println!("- {}", err.as_ref().unwrap_err());
        });

        if is_colored() {
            print!("Failure{}; ", RESET);
        } else {
            print!("Failure; ");
        }
    } else {
        if is_colored() {
            print!("{}Success{}; ", GREEN, RESET);
        } else {
            print!("Success; ");
        }
    }

    print!("Completed: {:?}; Failed: {:?}; ", completed, errors.len());
    print_time_elapsed(time_elapsed);
}

pub fn clear_screen() -> () {
    // See https://archive.ph/d3Z3O
    print!("\n{}[2J", 27 as char);
}
