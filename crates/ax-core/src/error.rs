/// Errors that can occur in backend operations.
#[derive(Debug, thiserror::Error)]
#[non_exhaustive]
pub enum BackendError {
    /// Path does not exist.
    #[error("Path not found: {0}")]
    NotFound(String),

    /// Path is not a directory (for list operations).
    #[error("Path is not a directory: {0}")]
    NotADirectory(String),

    /// Path traversal attempt detected.
    #[error("Path traversal attempt detected: {0}")]
    PathTraversal(String),

    /// Permission denied for the given path or operation.
    #[error("Permission denied: {0}")]
    PermissionDenied(String),

    /// Connection to a backend failed.
    #[error("Connection to backend '{backend}' failed")]
    ConnectionFailed {
        backend: String,
        #[source]
        source: Box<dyn std::error::Error + Send + Sync>,
    },

    /// Operation timed out.
    #[error("Operation '{operation}' timed out for path: {path}")]
    Timeout { operation: String, path: String },

    /// IO error.
    #[error("IO error: {0}")]
    Io(#[from] std::io::Error),

    /// Other backend-specific error.
    #[error("Backend error: {0}")]
    Other(String),
}

impl BackendError {
    /// Returns true if this error is transient and the operation may succeed on retry.
    pub fn is_transient(&self) -> bool {
        match self {
            BackendError::ConnectionFailed { .. } => true,
            BackendError::Timeout { .. } => true,
            BackendError::Io(e) => matches!(
                e.kind(),
                std::io::ErrorKind::ConnectionRefused
                    | std::io::ErrorKind::ConnectionReset
                    | std::io::ErrorKind::ConnectionAborted
                    | std::io::ErrorKind::TimedOut
                    | std::io::ErrorKind::Interrupted
            ),
            _ => false,
        }
    }
}

/// Errors that can occur during VFS operations.
#[derive(Debug, thiserror::Error)]
#[non_exhaustive]
pub enum VfsError {
    /// No mount found for the given path.
    #[error("No mount found for path '{0}'. Check your ax.yaml mounts configuration.")]
    NoMount(String),

    /// Attempted to write to a read-only mount.
    #[error("Mount is read-only: {0}. Remove 'read_only: true' from the mount config to enable writes.")]
    ReadOnly(String),

    /// Path does not exist.
    #[error("Path not found: {0}")]
    NotFound(String),

    /// Backend-specific error.
    #[error("Backend error: {0}")]
    Backend(Box<dyn std::error::Error + Send + Sync>),

    /// IO error.
    #[error("IO error: {0}")]
    Io(#[from] std::io::Error),

    /// Configuration error.
    #[error("Config error: {0}")]
    Config(String),

    /// Watch-related error.
    #[error("Watch error: {0}")]
    Watch(String),

    /// Indexing-related error.
    #[error("Indexing error: {0}")]
    Indexing(String),
}

impl From<BackendError> for VfsError {
    fn from(e: BackendError) -> Self {
        match e {
            BackendError::NotFound(path) => VfsError::NotFound(path),
            BackendError::Io(io_err) => VfsError::Io(io_err),
            _ => VfsError::Backend(Box::new(e)),
        }
    }
}

impl From<ax_config::ConfigError> for VfsError {
    fn from(e: ax_config::ConfigError) -> Self {
        VfsError::Config(e.to_string())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_backend_is_transient_connection_failed() {
        let err = BackendError::ConnectionFailed {
            backend: "s3".to_string(),
            source: Box::new(std::io::Error::other("conn err")),
        };
        assert!(err.is_transient());
    }

    #[test]
    fn test_backend_is_transient_timeout() {
        let err = BackendError::Timeout {
            operation: "read".to_string(),
            path: "/foo".to_string(),
        };
        assert!(err.is_transient());
    }

    #[test]
    fn test_backend_not_transient_not_found() {
        let err = BackendError::NotFound("/missing".to_string());
        assert!(!err.is_transient());
    }

    #[test]
    fn test_vfs_from_backend_not_found() {
        let backend_err = BackendError::NotFound("/missing".to_string());
        let vfs_err: VfsError = backend_err.into();
        assert!(matches!(vfs_err, VfsError::NotFound(p) if p == "/missing"));
    }

    #[test]
    fn test_vfs_from_backend_io() {
        let io_err = std::io::Error::new(std::io::ErrorKind::PermissionDenied, "denied");
        let backend_err = BackendError::Io(io_err);
        let vfs_err: VfsError = backend_err.into();
        assert!(matches!(vfs_err, VfsError::Io(_)));
    }

    #[test]
    fn test_vfs_from_backend_other() {
        let backend_err = BackendError::Other("something failed".to_string());
        let vfs_err: VfsError = backend_err.into();
        assert!(matches!(vfs_err, VfsError::Backend(_)));
    }

    #[test]
    fn test_vfs_from_config_error() {
        let config_err = ax_config::ConfigError::InvalidConfig("bad config".to_string());
        let vfs_err: VfsError = config_err.into();
        assert!(matches!(vfs_err, VfsError::Config(_)));
    }

    #[test]
    fn test_display_no_mount() {
        let err = VfsError::NoMount("/foo".to_string());
        let msg = err.to_string();
        assert!(msg.contains("/foo"));
        assert!(msg.contains("ax.yaml"));
    }
}
