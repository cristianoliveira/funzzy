pub fn info(msg: &str) {
    println!("Funzzy: {}", msg);
}

pub fn error(msg: &str) {
    println!("Funzzy error: {}", msg);
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
        println!("Failed tasks: {:?}", errors.len());
        errors.iter().for_each(|err| {
            println!(" - {}", err.as_ref().unwrap_err());
        });
    } else {
        println!("All tasks finished successfully.");
    }
}

pub fn clear_screen() -> () {
    // See https://archive.ph/d3Z3O
    print!("{}[2J", 27 as char);
}
