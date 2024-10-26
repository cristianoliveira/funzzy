use std::error::Error;
use std::fmt;

use yaml_rust::ScanError;

use crate::stdout;

pub type Hint = Option<String>;
pub type Result<T> = std::result::Result<T, FzzError>;

fn hint_formatter(hint: &str) -> String {
    format!("{}Hint{}: {}", stdout::BLUE, stdout::RESET, hint)
}

pub type UnkownError = Box<dyn Error + Send + Sync>;

#[derive(Debug)]
pub enum FzzError {
    IoConfigError(String, Option<std::io::Error>),
    IoStdinError(String, Hint),
    InvalidConfigError(String, Option<ScanError>, Hint),
    PathError(String, Option<UnkownError>, Hint),
    PathPatternError(String, Hint),
    GenericError(String),
}

impl fmt::Display for FzzError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            FzzError::IoConfigError(msg, Some(err)) => match err.kind() {
                std::io::ErrorKind::NotFound => {
                    let hints = "Check if the file exists and if the path is correct. Try `fzz init` to create a new configuration file";
                    write!(f, "{}\nReason: {}\n{}", msg, err, hint_formatter(hints))
                }
                std::io::ErrorKind::PermissionDenied => {
                    let hints = "Check if you have permission to write in the current folder";
                    write!(f, "{}\nReason: {}\n{}", msg, err, hint_formatter(hints))
                }
                _ => write!(f, "{}\nReason: {}", msg, err),
            },
            FzzError::IoConfigError(msg, _) => {
                write!(f, "{}", msg)
            }
            FzzError::IoStdinError(err, hints) => {
                if let Some(hints) = hints {
                    write!(f, "Reason: {}\n{}", err, hint_formatter(hints))
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
                    write!(f, "{}{}\n{}", msg, err, hint_formatter(hints))
                } else {
                    write!(f, "{}{}", msg, err)
                }
            }
            FzzError::PathPatternError(msg, hints) => {
                if let Some(hints) = hints {
                    write!(f, "{}\n{}", msg, hint_formatter(hints))
                } else {
                    write!(f, "{}", msg)
                }
            }
            FzzError::PathError(msg, error, hints) => {
                let info = if let Some(e) = error {
                    format!("{}\nReason: {}", msg, e)
                } else {
                    format!("{}", msg)
                };

                if let Some(hints) = hints {
                    write!(f, "{}\n{}", info, hint_formatter(hints))
                } else {
                    write!(f, "{}", info)
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
            FzzError::PathError(_, Some(e), _) => Some(e.as_ref()),
            _ => None,
        }
    }
}
