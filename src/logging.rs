use once_cell::sync::Lazy;
use std::borrow::Cow;
use std::fs::{File, OpenOptions};
use std::io::{self, Seek, SeekFrom, Write};
use std::path::PathBuf;
use std::sync::Mutex;

struct Logger {
    file: File,
}

static LOGGER: Lazy<Mutex<Option<Logger>>> = Lazy::new(|| Mutex::new(None));

pub fn init(path: PathBuf) -> io::Result<()> {
    let file = OpenOptions::new()
        .create(true)
        .write(true)
        .truncate(true)
        .open(path)?;

    let mut logger = LOGGER
        .lock()
        .map_err(|_| io::Error::new(io::ErrorKind::Other, "logger poisoned"))?;
    *logger = Some(Logger { file });

    Ok(())
}

pub fn is_enabled() -> bool {
    LOGGER
        .lock()
        .map(|logger| logger.is_some())
        .unwrap_or(false)
}

pub fn log_line(message: &str) {
    log_internal(message, true);
}

pub fn log_plain(message: &str) {
    log_internal(message, false);
}

pub fn truncate() -> io::Result<()> {
    let mut logger = LOGGER
        .lock()
        .map_err(|_| io::Error::new(io::ErrorKind::Other, "logger poisoned"))?;

    if let Some(ref mut logger) = *logger {
        logger.file.set_len(0)?;
        logger.file.seek(SeekFrom::Start(0))?;
    }

    Ok(())
}

fn log_internal(message: &str, newline: bool) {
    if let Ok(mut logger) = LOGGER.lock() {
        if let Some(ref mut logger) = *logger {
            let sanitized = sanitize(message);
            let res = if newline {
                writeln!(logger.file, "{}", sanitized)
            } else {
                write!(logger.file, "{}", sanitized)
            };

            if let Err(err) = res.and_then(|_| logger.file.flush()) {
                eprintln!("Funzzy logger error: {}", err);
            }
        }
    }
}

fn sanitize(message: &str) -> Cow<'_, str> {
    if !message.as_bytes().contains(&0x1b) {
        return Cow::Borrowed(message);
    }

    let mut output = Vec::with_capacity(message.len());
    let bytes = message.as_bytes();
    let mut i = 0;
    while i < bytes.len() {
        if bytes[i] == 0x1b {
            i += 1;
            if i < bytes.len() && bytes[i] == b'[' {
                i += 1;
                while i < bytes.len() && bytes[i] != b'm' {
                    i += 1;
                }
                if i < bytes.len() {
                    i += 1;
                }
                continue;
            }
        }

        output.push(bytes[i]);
        i += 1;
    }

    Cow::Owned(String::from_utf8_lossy(&output).into_owned())
}

#[cfg(test)]
mod tests {
    use super::sanitize;

    #[test]
    fn it_removes_ansi_escape_sequences() {
        let colored = "\x1b[31mFunzzy\x1b[0m: log";
        let sanitized = sanitize(colored);
        assert_eq!(sanitized, "Funzzy: log");
    }

    #[test]
    fn it_returns_original_when_no_sequences() {
        let plain = "Funzzy: log";
        let sanitized = sanitize(plain);
        assert!(matches!(sanitized, std::borrow::Cow::Borrowed(_)));
    }
}
