use std::io::Write;

use crate::environment;

// ANSI color codes for terminal output
pub const GREEN: &str = "\x1b[32m";
pub const RED: &str = "\x1b[31m";
pub const BLUE: &str = "\x1b[34m";
pub const RESET: &str = "\x1b[0m";

fn is_colored() -> bool {
    environment::is_enabled("FUNZZY_COLORED")
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
    print!("Finished in {:.4}s", elapsed.as_secs_f32());
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
    print!("Finished in 0.0s");
    std::io::stdout().flush().expect("Failed to flush stdout");
}

pub fn present_results(results: Vec<Result<(), String>>, time_elapsed: std::time::Duration) {
    let errors: Vec<Result<(), String>> = results.iter().cloned().filter(|r| r.is_err()).collect();
    println!("Funzzy results ----------------------------");
    if !errors.is_empty() {
        if is_colored() {
            print!("{}", RED);
        }
        println!("Failed tasks: {:?}", errors.len());
        errors.iter().for_each(|err| {
            println!(" - {}", err.as_ref().unwrap_err());
        });
    } else {
        if is_colored() {
            print!("{}", GREEN);
        }
        print!("All tasks finished successfully. ");
    }

    if is_colored() {
        print!("{}", RESET);
    }

    print_time_elapsed(time_elapsed);
}

pub fn clear_screen() -> () {
    // See https://archive.ph/d3Z3O
    print!("\n{}[2J", 27 as char);
}
