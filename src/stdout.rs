pub fn info(msg: &str) {
    println!("Funzzy: {}", msg);
}

pub fn error(msg: &str) {
    println!("Funzzy error: {}", msg);
}

pub fn verbose(msg: &str) {
    println!("-----------------------------");
    println!("Funzzy verbose: {} ", msg);
    println!("-----------------------------");
}
