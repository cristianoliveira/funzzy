use std::error::Error;
use std::fmt;

use yaml_rust::ScanError;

pub type Hint = Option<String>;
pub type Result<T> = std::result::Result<T, FzzError>;

#[derive(Debug)]
pub enum FzzError {
    IoConfigError(String, Option<std::io::Error>),
    IoStdinError(String, Hint),
    InvalidConfigError(String, Option<ScanError>, Hint),
    GenericError(String),
}

impl fmt::Display for FzzError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            FzzError::IoConfigError(msg, Some(err)) => match err.kind() {
                std::io::ErrorKind::NotFound => {
                    let hints = "Check if the file exists and if the path is correct";
                    write!(f, "{}\nReason: {}\nHint: {}", msg, err, hints)
                }
                std::io::ErrorKind::PermissionDenied => {
                    let hints = "Check if you have permission to write in the current folder";
                    write!(f, "{}\nReason: {}\nHint: {}", msg, err, hints)
                }
                _ => write!(f, "{}\nReason: {}", msg, err),
            },
            FzzError::IoConfigError(msg, _) => {
                write!(f, "{}", msg)
            }
            FzzError::IoStdinError(err, hints) => {
                if let Some(hints) = hints {
                    write!(f, "Reason: {}\nHint: {}", err, hints)
                } else {
                    write!(f, "Reason: {}", err)
                }
            }
            FzzError::InvalidConfigError(msg, error, hints) => {
                let err = match error {
                    Some(e) => format!("\nReason: {}", e),
                    _ => "".to_string(),
                };

                if let Some(hints) = hints {
                    write!(f, "{}{}\nHint: {}", msg, err, hints)
                } else {
                    write!(f, "{}{}", msg, err)
                }
            }
            FzzError::GenericError(e) => write!(f, "Reason: {}", e),
        }
    }
}

impl Error for FzzError {
    fn source(&self) -> Option<&(dyn Error + 'static)> {
        match self {
            FzzError::IoConfigError(_, Some(e)) => Some(e),
            _ => None,
        }
    }
}
