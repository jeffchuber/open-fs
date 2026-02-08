use std::fmt;

/// Errors that can occur during VFS operations.
#[derive(Debug)]
pub enum VfsError {
    /// No mount found for the given path.
    NoMount(String),

    /// Attempted to write to a read-only mount.
    ReadOnly(String),

    /// Path does not exist.
    NotFound(String),

    /// Backend-specific error.
    Backend(Box<dyn std::error::Error + Send + Sync>),

    /// IO error.
    Io(std::io::Error),

    /// Configuration error.
    Config(String),
}

impl fmt::Display for VfsError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            VfsError::NoMount(path) => write!(f, "No mount found for path: {}", path),
            VfsError::ReadOnly(path) => write!(f, "Mount is read-only: {}", path),
            VfsError::NotFound(path) => write!(f, "Path not found: {}", path),
            VfsError::Backend(e) => write!(f, "Backend error: {}", e),
            VfsError::Io(e) => write!(f, "IO error: {}", e),
            VfsError::Config(msg) => write!(f, "Config error: {}", msg),
        }
    }
}

impl std::error::Error for VfsError {
    fn source(&self) -> Option<&(dyn std::error::Error + 'static)> {
        match self {
            VfsError::Backend(e) => Some(e.as_ref()),
            VfsError::Io(e) => Some(e),
            _ => None,
        }
    }
}

impl From<std::io::Error> for VfsError {
    fn from(e: std::io::Error) -> Self {
        VfsError::Io(e)
    }
}

impl From<ax_config::ConfigError> for VfsError {
    fn from(e: ax_config::ConfigError) -> Self {
        VfsError::Config(e.to_string())
    }
}

impl From<ax_backends::BackendError> for VfsError {
    fn from(e: ax_backends::BackendError) -> Self {
        match e {
            ax_backends::BackendError::NotFound(path) => VfsError::NotFound(path),
            ax_backends::BackendError::Io(io_err) => VfsError::Io(io_err),
            _ => VfsError::Backend(Box::new(e)),
        }
    }
}
