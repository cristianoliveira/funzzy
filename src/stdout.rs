use std::io::Write;

// ANSI color codes for terminal output
pub const GREEN: &str = "\x1b[32m";
pub const RED: &str = "\x1b[31m";
pub const YELLOW: &str = "\x1b[33m";
pub const BLUE: &str = "\x1b[34m";
pub const RESET: &str = "\x1b[0m";

pub fn info(msg: &str) {
    println!("{}Funzzy{}: {}", BLUE, RESET, msg);
}

pub fn pinfo(msg: &str) {
    print!("{}Funzzy{}: {}", BLUE, RESET, msg);
    std::io::stdout().flush().expect("Failed to flush stdout");
}

pub fn error(msg: &str) {
    println!("{}Funzzy error{}: {}", RED, RESET, msg);
}

pub fn warn(msg: &str) {
    println!("{}Funzzy warning{}: {}", YELLOW, RESET, msg);
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
        println!("{}", RED);
        println!("Failed tasks: {:?}", errors.len());
        errors.iter().for_each(|err| {
            println!(" - {}", err.as_ref().unwrap_err());
        });
    } else {
        println!("{}", GREEN);
        println!("All tasks finished successfully.");
    }
    println!("{}", RESET);
}

pub fn clear_screen() -> () {
    // See https://archive.ph/d3Z3O
    print!("\n{}[2J", 27 as char);
}
