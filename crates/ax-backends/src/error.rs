use std::fmt;

/// Errors that can occur in backend operations.
#[derive(Debug)]
pub enum BackendError {
    /// Path does not exist.
    NotFound(String),

    /// Path is not a directory (for list operations).
    NotADirectory(String),

    /// Path traversal attempt detected.
    PathTraversal(String),

    /// IO error.
    Io(std::io::Error),

    /// Other backend-specific error.
    Other(String),
}

impl fmt::Display for BackendError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            BackendError::NotFound(path) => write!(f, "Path not found: {}", path),
            BackendError::NotADirectory(path) => write!(f, "Path is not a directory: {}", path),
            BackendError::PathTraversal(path) => {
                write!(f, "Path traversal attempt detected: {}", path)
            }
            BackendError::Io(e) => write!(f, "IO error: {}", e),
            BackendError::Other(msg) => write!(f, "Backend error: {}", msg),
        }
    }
}

impl std::error::Error for BackendError {
    fn source(&self) -> Option<&(dyn std::error::Error + 'static)> {
        match self {
            BackendError::Io(e) => Some(e),
            _ => None,
        }
    }
}

impl From<std::io::Error> for BackendError {
    fn from(e: std::io::Error) -> Self {
        BackendError::Io(e)
    }
}
