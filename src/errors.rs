use std::error::Error;
use std::fmt;

#[derive(Debug)]
pub enum FzzError {
    IoConfigError(String, Option<std::io::Error>),
    GenericError(String),
}

impl fmt::Display for FzzError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            FzzError::IoConfigError(msg, Some(err)) => {
                if err.kind() == std::io::ErrorKind::PermissionDenied {
                    let hints = "Check if you have permission to write in the current folder";
                    write!(f, "{}\nReason: {}\nHint: {}", msg, err, hints)
                } else {
                    write!(f, "{}\nReason: {}", msg, err)
                }
            }
            FzzError::IoConfigError(msg, _) => {
                write!(f, "{}", msg)
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
