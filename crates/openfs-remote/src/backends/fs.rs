use std::path::{Path, PathBuf};
use std::time::UNIX_EPOCH;

use async_trait::async_trait;
use chrono::{DateTime, Utc};
use std::path::Component;
use tokio::fs;
use tokio::io::AsyncWriteExt;
use tracing::{debug, instrument};

use openfs_core::{Backend, BackendError, Entry};

/// Local filesystem backend.
pub struct FsBackend {
    root: PathBuf,
}

impl FsBackend {
    /// Create a new filesystem backend rooted at the given path.
    pub fn new(root: impl AsRef<Path>) -> Result<Self, BackendError> {
        let root = root.as_ref();

        // Canonicalize if the path exists, otherwise just use the path as-is
        let root = if root.exists() {
            root.canonicalize().map_err(|e| {
                BackendError::Io(std::io::Error::new(
                    std::io::ErrorKind::InvalidInput,
                    format!("Failed to canonicalize root path: {}", e),
                ))
            })?
        } else {
            // Create the directory if it doesn't exist
            std::fs::create_dir_all(root).map_err(BackendError::Io)?;
            root.canonicalize().map_err(BackendError::Io)?
        };

        Ok(FsBackend { root })
    }

    /// Resolve a relative path to an absolute path, preventing directory traversal.
    fn resolve_path(&self, path: &str) -> Result<PathBuf, BackendError> {
        let trimmed = path.trim_start_matches('/');
        let rel = Path::new(trimmed);

        // Reject attempts to traverse outside the root.
        for component in rel.components() {
            match component {
                Component::ParentDir | Component::RootDir | Component::Prefix(_) => {
                    return Err(BackendError::PathTraversal(trimmed.to_string()));
                }
                _ => {}
            }
        }

        let full_path = self.root.join(rel);

        // Find the nearest existing ancestor and ensure it resolves under root.
        let mut ancestor = full_path.as_path();
        while !ancestor.exists() {
            if let Some(parent) = ancestor.parent() {
                ancestor = parent;
            } else {
                break;
            }
        }

        let canonical_ancestor = ancestor.canonicalize().map_err(BackendError::Io)?;
        if !canonical_ancestor.starts_with(&self.root) {
            return Err(BackendError::PathTraversal(trimmed.to_string()));
        }

        Ok(full_path)
    }

    fn cas_token_for_metadata(metadata: &std::fs::Metadata) -> String {
        let modified_ns = metadata
            .modified()
            .ok()
            .and_then(|time| time.duration_since(UNIX_EPOCH).ok())
            .map(|duration| duration.as_nanos())
            .unwrap_or(0);
        format!("{}:{}", metadata.len(), modified_ns)
    }
}

#[async_trait]
impl Backend for FsBackend {
    #[instrument(skip(self), fields(backend = "fs", path = %path))]
    async fn read(&self, path: &str) -> Result<Vec<u8>, BackendError> {
        let full_path = self.resolve_path(path)?;
        debug!(full_path = ?full_path, "reading file");
        fs::read(&full_path).await.map_err(|e| {
            if e.kind() == std::io::ErrorKind::NotFound {
                BackendError::NotFound(path.to_string())
            } else {
                BackendError::Io(e)
            }
        })
    }

    #[instrument(skip(self, content), fields(backend = "fs", path = %path, size = content.len()))]
    async fn write(&self, path: &str, content: &[u8]) -> Result<(), BackendError> {
        let full_path = self.resolve_path(path)?;
        debug!(full_path = ?full_path, "writing file");

        // Create parent directories if needed
        if let Some(parent) = full_path.parent() {
            fs::create_dir_all(parent).await.map_err(BackendError::Io)?;
        }

        fs::write(&full_path, content)
            .await
            .map_err(BackendError::Io)
    }

    async fn read_with_cas_token(
        &self,
        path: &str,
    ) -> Result<(Vec<u8>, Option<String>), BackendError> {
        let full_path = self.resolve_path(path)?;
        let content = self.read(path).await?;
        let metadata = fs::metadata(&full_path).await.map_err(|e| {
            if e.kind() == std::io::ErrorKind::NotFound {
                BackendError::NotFound(path.to_string())
            } else {
                BackendError::Io(e)
            }
        })?;
        Ok((content, Some(Self::cas_token_for_metadata(&metadata))))
    }

    async fn compare_and_swap(
        &self,
        path: &str,
        expected: Option<&str>,
        content: &[u8],
    ) -> Result<Option<String>, BackendError> {
        let full_path = self.resolve_path(path)?;

        if let Some(expected_token) = expected {
            let actual_token = match fs::metadata(&full_path).await {
                Ok(metadata) => Some(Self::cas_token_for_metadata(&metadata)),
                Err(e) if e.kind() == std::io::ErrorKind::NotFound => None,
                Err(e) => return Err(BackendError::Io(e)),
            };

            if actual_token.as_deref() != Some(expected_token) {
                return Err(BackendError::PreconditionFailed {
                    path: path.to_string(),
                    expected: expected_token.to_string(),
                    actual: actual_token.unwrap_or_else(|| "absent".to_string()),
                });
            }
        }

        if let Some(parent) = full_path.parent() {
            fs::create_dir_all(parent).await.map_err(BackendError::Io)?;
        }
        fs::write(&full_path, content)
            .await
            .map_err(BackendError::Io)?;

        let metadata = fs::metadata(&full_path).await.map_err(BackendError::Io)?;
        Ok(Some(Self::cas_token_for_metadata(&metadata)))
    }

    #[instrument(skip(self, content), fields(backend = "fs", path = %path, size = content.len()))]
    async fn append(&self, path: &str, content: &[u8]) -> Result<(), BackendError> {
        let full_path = self.resolve_path(path)?;
        debug!(full_path = ?full_path, "appending to file");

        // Create parent directories if needed
        if let Some(parent) = full_path.parent() {
            fs::create_dir_all(parent).await.map_err(BackendError::Io)?;
        }

        let mut file = fs::OpenOptions::new()
            .create(true)
            .append(true)
            .open(&full_path)
            .await
            .map_err(BackendError::Io)?;

        file.write_all(content).await.map_err(BackendError::Io)
    }

    #[instrument(skip(self), fields(backend = "fs", path = %path))]
    async fn delete(&self, path: &str) -> Result<(), BackendError> {
        let full_path = self.resolve_path(path)?;
        debug!(full_path = ?full_path, "deleting file");

        if full_path.is_dir() {
            fs::remove_dir_all(&full_path)
                .await
                .map_err(BackendError::Io)
        } else {
            fs::remove_file(&full_path).await.map_err(|e| {
                if e.kind() == std::io::ErrorKind::NotFound {
                    BackendError::NotFound(path.to_string())
                } else {
                    BackendError::Io(e)
                }
            })
        }
    }

    #[instrument(skip(self), fields(backend = "fs", path = %path))]
    async fn list(&self, path: &str) -> Result<Vec<Entry>, BackendError> {
        let full_path = self.resolve_path(path)?;

        // If path is empty, list the root
        let full_path = if path.is_empty() || path == "/" {
            self.root.clone()
        } else {
            full_path
        };

        if !full_path.exists() {
            return Err(BackendError::NotFound(path.to_string()));
        }

        if !full_path.is_dir() {
            return Err(BackendError::NotADirectory(path.to_string()));
        }

        let mut entries = Vec::new();
        let mut read_dir = fs::read_dir(&full_path).await.map_err(BackendError::Io)?;

        while let Some(entry) = read_dir.next_entry().await.map_err(BackendError::Io)? {
            let metadata = entry.metadata().await.map_err(BackendError::Io)?;
            let name = entry.file_name().to_string_lossy().to_string();

            let entry_path = if path.is_empty() || path == "/" {
                format!("/{}", name)
            } else {
                format!("{}/{}", path.trim_end_matches('/'), name)
            };

            let modified = metadata.modified().ok().map(DateTime::<Utc>::from);

            if metadata.is_dir() {
                entries.push(Entry::dir(entry_path, name, modified));
            } else {
                entries.push(Entry::file(entry_path, name, metadata.len(), modified));
            }
        }

        // Sort entries: directories first, then alphabetically
        entries.sort_by(|a, b| match (a.is_dir, b.is_dir) {
            (true, false) => std::cmp::Ordering::Less,
            (false, true) => std::cmp::Ordering::Greater,
            _ => a.name.cmp(&b.name),
        });

        Ok(entries)
    }

    #[instrument(skip(self), fields(backend = "fs", path = %path))]
    async fn exists(&self, path: &str) -> Result<bool, BackendError> {
        let full_path = self.resolve_path(path)?;
        Ok(full_path.exists())
    }

    #[instrument(skip(self), fields(backend = "fs", path = %path))]
    async fn stat(&self, path: &str) -> Result<Entry, BackendError> {
        let full_path = self.resolve_path(path)?;

        let metadata = fs::metadata(&full_path).await.map_err(|e| {
            if e.kind() == std::io::ErrorKind::NotFound {
                BackendError::NotFound(path.to_string())
            } else {
                BackendError::Io(e)
            }
        })?;

        let name = full_path
            .file_name()
            .map(|n| n.to_string_lossy().to_string())
            .unwrap_or_default();

        let modified = metadata.modified().ok().map(DateTime::<Utc>::from);

        if metadata.is_dir() {
            Ok(Entry::dir(path.to_string(), name, modified))
        } else {
            Ok(Entry::file(
                path.to_string(),
                name,
                metadata.len(),
                modified,
            ))
        }
    }

    #[instrument(skip(self), fields(backend = "fs", from = %from, to = %to))]
    async fn rename(&self, from: &str, to: &str) -> Result<(), BackendError> {
        let from_path = self.resolve_path(from)?;
        let to_path = self.resolve_path(to)?;
        debug!(from = ?from_path, to = ?to_path, "renaming file");

        // Create parent directories for destination if needed
        if let Some(parent) = to_path.parent() {
            fs::create_dir_all(parent).await.map_err(BackendError::Io)?;
        }

        fs::rename(&from_path, &to_path).await.map_err(|e| {
            if e.kind() == std::io::ErrorKind::NotFound {
                BackendError::NotFound(from.to_string())
            } else {
                BackendError::Io(e)
            }
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::TempDir;

    #[cfg(unix)]
    use std::os::unix::fs as unix_fs;

    #[tokio::test]
    async fn test_write_and_read() {
        let temp_dir = TempDir::new().unwrap();
        let backend = FsBackend::new(temp_dir.path()).unwrap();

        backend.write("test.txt", b"hello world").await.unwrap();
        let content = backend.read("test.txt").await.unwrap();
        assert_eq!(content, b"hello world");
    }

    #[tokio::test]
    async fn test_list() {
        let temp_dir = TempDir::new().unwrap();
        let backend = FsBackend::new(temp_dir.path()).unwrap();

        backend.write("file1.txt", b"content1").await.unwrap();
        backend.write("file2.txt", b"content2").await.unwrap();
        backend
            .write("subdir/file3.txt", b"content3")
            .await
            .unwrap();

        let entries = backend.list("").await.unwrap();
        assert_eq!(entries.len(), 3); // subdir, file1.txt, file2.txt

        let names: Vec<_> = entries.iter().map(|e| e.name.as_str()).collect();
        assert!(names.contains(&"file1.txt"));
        assert!(names.contains(&"file2.txt"));
        assert!(names.contains(&"subdir"));
    }

    #[tokio::test]
    async fn test_delete() {
        let temp_dir = TempDir::new().unwrap();
        let backend = FsBackend::new(temp_dir.path()).unwrap();

        backend.write("test.txt", b"hello").await.unwrap();
        assert!(backend.exists("test.txt").await.unwrap());

        backend.delete("test.txt").await.unwrap();
        assert!(!backend.exists("test.txt").await.unwrap());
    }

    #[tokio::test]
    async fn test_path_traversal_rejected() {
        let temp_dir = TempDir::new().unwrap();
        let backend = FsBackend::new(temp_dir.path()).unwrap();

        let err = backend.write("../escape.txt", b"nope").await.unwrap_err();
        assert!(matches!(err, BackendError::PathTraversal(_)));
    }

    #[cfg(unix)]
    #[tokio::test]
    async fn test_symlink_escape_rejected() {
        let temp_dir = TempDir::new().unwrap();
        let backend = FsBackend::new(temp_dir.path()).unwrap();

        let outside_dir = TempDir::new().unwrap();
        let link_path = temp_dir.path().join("escape");
        unix_fs::symlink(outside_dir.path(), &link_path).unwrap();

        let err = backend.write("escape/evil.txt", b"nope").await.unwrap_err();
        assert!(matches!(err, BackendError::PathTraversal(_)));
    }

    #[tokio::test]
    async fn test_compare_and_swap_success_and_mismatch() {
        let temp_dir = TempDir::new().unwrap();
        let backend = FsBackend::new(temp_dir.path()).unwrap();

        backend.write("cas.txt", b"v1").await.unwrap();
        let (_content, token) = backend.read_with_cas_token("cas.txt").await.unwrap();
        let token = token.expect("token should be present");

        let new_token = backend
            .compare_and_swap("cas.txt", Some(&token), b"v2")
            .await
            .expect("cas write should succeed")
            .expect("new token should be present");
        assert_ne!(token, new_token);

        let err = backend
            .compare_and_swap("cas.txt", Some(&token), b"v3")
            .await
            .unwrap_err();
        assert!(matches!(err, BackendError::PreconditionFailed { .. }));
    }
}
