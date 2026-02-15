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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_is_transient_connection_failed() {
        let err = BackendError::ConnectionFailed {
            backend: "s3".to_string(),
            source: Box::new(std::io::Error::other("conn err")),
        };
        assert!(err.is_transient());
    }

    #[test]
    fn test_is_transient_timeout() {
        let err = BackendError::Timeout {
            operation: "read".to_string(),
            path: "/foo".to_string(),
        };
        assert!(err.is_transient());
    }

    #[test]
    fn test_is_transient_io_connection_refused() {
        let err = BackendError::Io(std::io::Error::new(
            std::io::ErrorKind::ConnectionRefused,
            "refused",
        ));
        assert!(err.is_transient());
    }

    #[test]
    fn test_is_transient_io_timed_out() {
        let err = BackendError::Io(std::io::Error::new(
            std::io::ErrorKind::TimedOut,
            "timed out",
        ));
        assert!(err.is_transient());
    }

    #[test]
    fn test_not_transient_not_found() {
        let err = BackendError::NotFound("/missing".to_string());
        assert!(!err.is_transient());
    }

    #[test]
    fn test_not_transient_permission_denied() {
        let err = BackendError::PermissionDenied("/secret".to_string());
        assert!(!err.is_transient());
    }

    #[test]
    fn test_not_transient_path_traversal() {
        let err = BackendError::PathTraversal("../../etc/passwd".to_string());
        assert!(!err.is_transient());
    }

    #[test]
    fn test_not_transient_other() {
        let err = BackendError::Other("unknown".to_string());
        assert!(!err.is_transient());
    }

    #[test]
    fn test_not_transient_io_not_found() {
        let err = BackendError::Io(std::io::Error::new(
            std::io::ErrorKind::NotFound,
            "not found",
        ));
        assert!(!err.is_transient());
    }
}
