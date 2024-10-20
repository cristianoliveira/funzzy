use std::io::Write;

// ANSI color codes for terminal output
pub const GREEN: &str = "\x1b[32m";
pub const RED: &str = "\x1b[31m";
pub const BLUE: &str = "\x1b[34m";
pub const RESET: &str = "\x1b[0m";

fn is_color_enabled() -> bool {
    // Colour output is not enabled by default
    std::env::var("FUNZZY_RESULT_COLORED").is_ok()
}

pub fn info(msg: &str) {
    println!("Funzzy: {}", msg);
}

pub fn pinfo(msg: &str) {
    print!("Funzzy: {}", msg);
    std::io::stdout().flush().expect("Failed to flush stdout");
}

pub fn error(msg: &str) {
    println!("{}Funzzy error{}: {}", RED, RESET, msg);
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
        if is_color_enabled() {
            print!("{}", RED);
        }
        println!("Failed tasks: {:?}", errors.len());
        errors.iter().for_each(|err| {
            println!(" - {}", err.as_ref().unwrap_err());
        });
    } else {
        if is_color_enabled() {
            print!("{}", GREEN);
        }
        println!("All tasks finished successfully.");
    }

    if is_color_enabled() {
        print!("{}", RESET);
    }
}

pub fn clear_screen() -> () {
    // See https://archive.ph/d3Z3O
    print!("\n{}[2J", 27 as char);
}
