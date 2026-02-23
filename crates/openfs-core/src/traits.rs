use async_trait::async_trait;
use chrono::{DateTime, Utc};

use crate::error::BackendError;

/// Metadata about a file or directory entry.
#[derive(Debug, Clone)]
pub struct Entry {
    /// Full path of the entry.
    pub path: String,
    /// Name of the entry (filename or directory name).
    pub name: String,
    /// Whether this entry is a directory.
    pub is_dir: bool,
    /// Size in bytes (None for directories).
    pub size: Option<u64>,
    /// Last modification time.
    pub modified: Option<DateTime<Utc>>,
}

impl Entry {
    /// Create a new file entry.
    pub fn file(path: String, name: String, size: u64, modified: Option<DateTime<Utc>>) -> Self {
        Entry {
            path,
            name,
            is_dir: false,
            size: Some(size),
            modified,
        }
    }

    /// Create a new directory entry.
    pub fn dir(path: String, name: String, modified: Option<DateTime<Utc>>) -> Self {
        Entry {
            path,
            name,
            is_dir: true,
            size: None,
            modified,
        }
    }
}

/// Trait for VFS backend implementations.
#[async_trait]
pub trait Backend: Send + Sync + 'static {
    /// Read the contents of a file.
    async fn read(&self, path: &str) -> Result<Vec<u8>, BackendError>;

    /// Read a file and return an optional CAS token for conditional writes.
    ///
    /// Backends that support optimistic concurrency should return a stable token
    /// that can be passed to [`Backend::compare_and_swap`].
    async fn read_with_cas_token(
        &self,
        path: &str,
    ) -> Result<(Vec<u8>, Option<String>), BackendError> {
        let content = self.read(path).await?;
        Ok((content, None))
    }

    /// Write content to a file, creating it if it doesn't exist.
    async fn write(&self, path: &str, content: &[u8]) -> Result<(), BackendError>;

    /// Conditionally write content when `expected` matches the current CAS token.
    ///
    /// Backends that do not support CAS can still accept unconditional writes by
    /// passing `expected = None`.
    async fn compare_and_swap(
        &self,
        path: &str,
        expected: Option<&str>,
        content: &[u8],
    ) -> Result<Option<String>, BackendError> {
        if expected.is_some() {
            return Err(BackendError::Other(
                "compare_and_swap is not supported by this backend".to_string(),
            ));
        }
        self.write(path, content).await?;
        Ok(None)
    }

    /// Append content to a file.
    async fn append(&self, path: &str, content: &[u8]) -> Result<(), BackendError>;

    /// Delete a file.
    async fn delete(&self, path: &str) -> Result<(), BackendError>;

    /// List entries in a directory.
    async fn list(&self, path: &str) -> Result<Vec<Entry>, BackendError>;

    /// Check if a path exists.
    async fn exists(&self, path: &str) -> Result<bool, BackendError>;

    /// Get metadata for a path.
    async fn stat(&self, path: &str) -> Result<Entry, BackendError>;

    /// Rename/move a file or directory.
    ///
    /// Default implementation uses read-write-delete which is not atomic.
    /// Backends should override this with native rename when available.
    async fn rename(&self, from: &str, to: &str) -> Result<(), BackendError> {
        let content = self.read(from).await?;
        self.write(to, &content).await?;
        self.delete(from).await?;
        Ok(())
    }
}
