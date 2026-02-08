use async_trait::async_trait;
use chrono::{DateTime, Utc};

use crate::error::VfsError;

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

/// Convert from ax_backends::Entry to our Entry.
impl From<ax_backends::Entry> for Entry {
    fn from(e: ax_backends::Entry) -> Self {
        Entry {
            path: e.path,
            name: e.name,
            is_dir: e.is_dir,
            size: e.size,
            modified: e.modified,
        }
    }
}

/// Trait for VFS backend implementations.
#[async_trait]
pub trait Backend: Send + Sync + 'static {
    /// Read the contents of a file.
    async fn read(&self, path: &str) -> Result<Vec<u8>, VfsError>;

    /// Write content to a file, creating it if it doesn't exist.
    async fn write(&self, path: &str, content: &[u8]) -> Result<(), VfsError>;

    /// Append content to a file.
    async fn append(&self, path: &str, content: &[u8]) -> Result<(), VfsError>;

    /// Delete a file.
    async fn delete(&self, path: &str) -> Result<(), VfsError>;

    /// List entries in a directory.
    async fn list(&self, path: &str) -> Result<Vec<Entry>, VfsError>;

    /// Check if a path exists.
    async fn exists(&self, path: &str) -> Result<bool, VfsError>;

    /// Get metadata for a path.
    async fn stat(&self, path: &str) -> Result<Entry, VfsError>;
}
