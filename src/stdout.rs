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
        print!("{}", BLUE);
    }
    print!("Funzzy: ");
    if is_colored() {
        print!("{}", RESET);
    }

    println!("{}", msg);
}

pub fn pinfo(msg: &str) {
    if is_colored() {
        print!("{}", BLUE);
    }
    print!("Funzzy: ");
    if is_colored() {
        print!("{}", RESET);
    }
    print!("{}", msg);
    std::io::stdout().flush().expect("Failed to flush stdout");
}

pub fn error(msg: &str) {
    if is_colored() {
        print!("{}", RED);
    }
    print!("Funzzy error: ");
    if is_colored() {
        print!("{}", RESET);
    }
    println!(" {}", msg);
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

pub fn present_results(results: Vec<Result<(), String>>) {
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
        println!("All tasks finished successfully.");
    }

    if is_colored() {
        print!("{}", RESET);
    }
}

pub fn clear_screen() -> () {
    // See https://archive.ph/d3Z3O
    print!("\n{}[2J", 27 as char);
}
