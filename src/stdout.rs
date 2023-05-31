pub fn info(msg: &str) {
    println!("Funzzy: {}", msg);
}

pub fn error(msg: &str) {
    println!("Funzzy ERROR: {}", msg);
}

pub fn verbose(msg: &str, verbose: bool) {
    if !verbose {
        return;
    }

    println!("-----------------------------");
    println!("Funzzy verbose: {} ", msg);
    println!("-----------------------------");
}
