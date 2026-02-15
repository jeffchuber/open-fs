use std::collections::HashMap;
use std::sync::RwLock;

use async_trait::async_trait;
use chrono::{DateTime, Utc};

use crate::error::BackendError;
use crate::traits::{Backend, Entry};

/// In-memory backend for testing.
pub struct MemoryBackend {
    files: RwLock<HashMap<String, (Vec<u8>, DateTime<Utc>)>>,
}

impl MemoryBackend {
    /// Create a new empty memory backend.
    pub fn new() -> Self {
        MemoryBackend {
            files: RwLock::new(HashMap::new()),
        }
    }
}

impl Default for MemoryBackend {
    fn default() -> Self {
        Self::new()
    }
}

#[async_trait]
impl Backend for MemoryBackend {
    async fn read(&self, path: &str) -> Result<Vec<u8>, BackendError> {
        let files = self.files.read().unwrap_or_else(|e| e.into_inner());
        let normalized = normalize_path(path);

        files
            .get(&normalized)
            .map(|(content, _)| content.clone())
            .ok_or_else(|| BackendError::NotFound(path.to_string()))
    }

    async fn write(&self, path: &str, content: &[u8]) -> Result<(), BackendError> {
        let mut files = self.files.write().unwrap_or_else(|e| e.into_inner());
        let normalized = normalize_path(path);
        files.insert(normalized, (content.to_vec(), Utc::now()));
        Ok(())
    }

    async fn append(&self, path: &str, content: &[u8]) -> Result<(), BackendError> {
        let mut files = self.files.write().unwrap_or_else(|e| e.into_inner());
        let normalized = normalize_path(path);

        let entry = files.entry(normalized).or_insert_with(|| (Vec::new(), Utc::now()));
        entry.0.extend_from_slice(content);
        entry.1 = Utc::now();
        Ok(())
    }

    async fn delete(&self, path: &str) -> Result<(), BackendError> {
        let mut files = self.files.write().unwrap_or_else(|e| e.into_inner());
        let normalized = normalize_path(path);

        if files.remove(&normalized).is_some() {
            Ok(())
        } else {
            Err(BackendError::NotFound(path.to_string()))
        }
    }

    async fn list(&self, path: &str) -> Result<Vec<Entry>, BackendError> {
        let files = self.files.read().unwrap_or_else(|e| e.into_inner());
        let normalized = normalize_path(path);
        let prefix = if normalized.is_empty() {
            String::new()
        } else {
            format!("{}/", normalized)
        };

        let mut entries = HashMap::new();

        for (file_path, (content, mtime)) in files.iter() {
            let relative = if prefix.is_empty() {
                file_path.clone()
            } else if file_path.starts_with(&prefix) {
                file_path[prefix.len()..].to_string()
            } else {
                continue;
            };

            if relative.is_empty() {
                continue;
            }

            // Get the first component (file or directory)
            let first_component = relative.split('/').next().unwrap();

            if relative.contains('/') {
                // It's a directory
                entries.entry(first_component.to_string()).or_insert_with(|| {
                    Entry::dir(
                        format!("{}{}", if prefix.is_empty() { "" } else { &prefix }, first_component),
                        first_component.to_string(),
                        None,
                    )
                });
            } else {
                // It's a file
                entries.insert(
                    first_component.to_string(),
                    Entry::file(
                        format!("{}{}", if prefix.is_empty() { "" } else { &prefix }, first_component),
                        first_component.to_string(),
                        content.len() as u64,
                        Some(*mtime),
                    ),
                );
            }
        }

        let mut result: Vec<_> = entries.into_values().collect();
        result.sort_by(|a, b| {
            match (a.is_dir, b.is_dir) {
                (true, false) => std::cmp::Ordering::Less,
                (false, true) => std::cmp::Ordering::Greater,
                _ => a.name.cmp(&b.name),
            }
        });

        Ok(result)
    }

    async fn exists(&self, path: &str) -> Result<bool, BackendError> {
        let files = self.files.read().unwrap_or_else(|e| e.into_inner());
        let normalized = normalize_path(path);

        // Check for exact file match
        if files.contains_key(&normalized) {
            return Ok(true);
        }

        // Check for directory (any file with this prefix)
        let dir_prefix = format!("{}/", normalized);
        Ok(files.keys().any(|k| k.starts_with(&dir_prefix)))
    }

    async fn stat(&self, path: &str) -> Result<Entry, BackendError> {
        let files = self.files.read().unwrap_or_else(|e| e.into_inner());
        let normalized = normalize_path(path);

        // Check for exact file match
        if let Some((content, mtime)) = files.get(&normalized) {
            let name = normalized.rsplit('/').next().unwrap_or(&normalized);
            return Ok(Entry::file(
                normalized.clone(),
                name.to_string(),
                content.len() as u64,
                Some(*mtime),
            ));
        }

        // Check for directory
        let dir_prefix = format!("{}/", normalized);
        if files.keys().any(|k| k.starts_with(&dir_prefix)) {
            let name = normalized.rsplit('/').next().unwrap_or(&normalized).to_string();
            return Ok(Entry::dir(normalized, name, None));
        }

        Err(BackendError::NotFound(path.to_string()))
    }

    async fn rename(&self, from: &str, to: &str) -> Result<(), BackendError> {
        let mut files = self.files.write().unwrap_or_else(|e| e.into_inner());
        let from_normalized = normalize_path(from);
        let to_normalized = normalize_path(to);

        let entry = files
            .remove(&from_normalized)
            .ok_or_else(|| BackendError::NotFound(from.to_string()))?;

        files.insert(to_normalized, entry);
        Ok(())
    }
}

/// Normalize a path by removing leading/trailing slashes.
fn normalize_path(path: &str) -> String {
    path.trim_matches('/').to_string()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn test_write_and_read() {
        let backend = MemoryBackend::new();

        backend.write("test.txt", b"hello world").await.unwrap();
        let content = backend.read("test.txt").await.unwrap();
        assert_eq!(content, b"hello world");
    }

    #[tokio::test]
    async fn test_list() {
        let backend = MemoryBackend::new();

        backend.write("file1.txt", b"content1").await.unwrap();
        backend.write("file2.txt", b"content2").await.unwrap();
        backend.write("subdir/file3.txt", b"content3").await.unwrap();

        let entries = backend.list("").await.unwrap();
        assert_eq!(entries.len(), 3);

        let names: Vec<_> = entries.iter().map(|e| e.name.as_str()).collect();
        assert!(names.contains(&"file1.txt"));
        assert!(names.contains(&"file2.txt"));
        assert!(names.contains(&"subdir"));
    }

    #[tokio::test]
    async fn test_delete() {
        let backend = MemoryBackend::new();

        backend.write("test.txt", b"hello").await.unwrap();
        assert!(backend.exists("test.txt").await.unwrap());

        backend.delete("test.txt").await.unwrap();
        assert!(!backend.exists("test.txt").await.unwrap());
    }

    #[tokio::test]
    async fn test_append() {
        let backend = MemoryBackend::new();

        backend.write("test.txt", b"hello").await.unwrap();
        backend.append("test.txt", b" world").await.unwrap();

        let content = backend.read("test.txt").await.unwrap();
        assert_eq!(content, b"hello world");
    }

    #[tokio::test]
    async fn test_read_nonexistent() {
        let backend = MemoryBackend::new();
        let result = backend.read("nonexistent.txt").await;
        assert!(result.is_err());
        assert!(matches!(result.unwrap_err(), BackendError::NotFound(_)));
    }

    #[tokio::test]
    async fn test_delete_nonexistent() {
        let backend = MemoryBackend::new();
        let result = backend.delete("nonexistent.txt").await;
        assert!(result.is_err());
        assert!(matches!(result.unwrap_err(), BackendError::NotFound(_)));
    }

    #[tokio::test]
    async fn test_write_empty_content() {
        let backend = MemoryBackend::new();
        backend.write("empty.txt", b"").await.unwrap();
        let content = backend.read("empty.txt").await.unwrap();
        assert!(content.is_empty());
    }

    #[tokio::test]
    async fn test_write_binary_content() {
        let backend = MemoryBackend::new();
        let binary_data: Vec<u8> = (0..=255).collect();
        backend.write("binary.bin", &binary_data).await.unwrap();
        let content = backend.read("binary.bin").await.unwrap();
        assert_eq!(content, binary_data);
    }

    #[tokio::test]
    async fn test_overwrite_file() {
        let backend = MemoryBackend::new();
        backend.write("test.txt", b"original").await.unwrap();
        backend.write("test.txt", b"modified").await.unwrap();
        let content = backend.read("test.txt").await.unwrap();
        assert_eq!(content, b"modified");
    }

    #[tokio::test]
    async fn test_append_to_nonexistent_creates_file() {
        let backend = MemoryBackend::new();
        backend.append("new.txt", b"content").await.unwrap();
        let content = backend.read("new.txt").await.unwrap();
        assert_eq!(content, b"content");
    }

    #[tokio::test]
    async fn test_append_multiple_times() {
        let backend = MemoryBackend::new();
        backend.append("multi.txt", b"one").await.unwrap();
        backend.append("multi.txt", b"two").await.unwrap();
        backend.append("multi.txt", b"three").await.unwrap();
        let content = backend.read("multi.txt").await.unwrap();
        assert_eq!(content, b"onetwothree");
    }

    #[tokio::test]
    async fn test_list_empty_directory() {
        let backend = MemoryBackend::new();
        let entries = backend.list("").await.unwrap();
        assert!(entries.is_empty());
    }

    #[tokio::test]
    async fn test_list_nested_directories() {
        let backend = MemoryBackend::new();
        backend.write("a/b/c/deep.txt", b"deep").await.unwrap();
        backend.write("a/b/shallow.txt", b"shallow").await.unwrap();
        backend.write("a/top.txt", b"top").await.unwrap();

        // List root of "a"
        let entries = backend.list("a").await.unwrap();
        let names: Vec<_> = entries.iter().map(|e| e.name.as_str()).collect();
        assert!(names.contains(&"b"));
        assert!(names.contains(&"top.txt"));
        assert_eq!(entries.len(), 2);

        // List "a/b"
        let entries = backend.list("a/b").await.unwrap();
        let names: Vec<_> = entries.iter().map(|e| e.name.as_str()).collect();
        assert!(names.contains(&"c"));
        assert!(names.contains(&"shallow.txt"));
    }

    #[tokio::test]
    async fn test_exists_directory() {
        let backend = MemoryBackend::new();
        backend.write("dir/file.txt", b"content").await.unwrap();
        assert!(backend.exists("dir").await.unwrap());
        assert!(backend.exists("dir/file.txt").await.unwrap());
        assert!(!backend.exists("nonexistent").await.unwrap());
    }

    #[tokio::test]
    async fn test_stat_file() {
        let backend = MemoryBackend::new();
        backend.write("test.txt", b"hello").await.unwrap();
        let entry = backend.stat("test.txt").await.unwrap();
        assert_eq!(entry.name, "test.txt");
        assert!(!entry.is_dir);
        assert_eq!(entry.size, Some(5));
    }

    #[tokio::test]
    async fn test_stat_directory() {
        let backend = MemoryBackend::new();
        backend.write("dir/file.txt", b"content").await.unwrap();
        let entry = backend.stat("dir").await.unwrap();
        assert_eq!(entry.name, "dir");
        assert!(entry.is_dir);
    }

    #[tokio::test]
    async fn test_stat_nonexistent() {
        let backend = MemoryBackend::new();
        let result = backend.stat("nonexistent").await;
        assert!(result.is_err());
    }

    #[tokio::test]
    async fn test_path_normalization_leading_slash() {
        let backend = MemoryBackend::new();
        backend.write("/test.txt", b"content").await.unwrap();
        // Should be accessible without leading slash
        let content = backend.read("test.txt").await.unwrap();
        assert_eq!(content, b"content");
    }

    #[tokio::test]
    async fn test_path_normalization_trailing_slash() {
        let backend = MemoryBackend::new();
        backend.write("test.txt/", b"content").await.unwrap();
        // Should be accessible without trailing slash
        let content = backend.read("test.txt").await.unwrap();
        assert_eq!(content, b"content");
    }

    #[tokio::test]
    async fn test_list_sorting_dirs_first() {
        let backend = MemoryBackend::new();
        backend.write("z_file.txt", b"content").await.unwrap();
        backend.write("a_dir/file.txt", b"content").await.unwrap();
        backend.write("a_file.txt", b"content").await.unwrap();

        let entries = backend.list("").await.unwrap();
        // Directory should come first
        assert!(entries[0].is_dir);
        assert_eq!(entries[0].name, "a_dir");
    }

    #[tokio::test]
    async fn test_large_file() {
        let backend = MemoryBackend::new();
        let large_data = vec![0u8; 1024 * 1024]; // 1MB
        backend.write("large.bin", &large_data).await.unwrap();
        let content = backend.read("large.bin").await.unwrap();
        assert_eq!(content.len(), 1024 * 1024);
    }

    #[tokio::test]
    async fn test_unicode_filenames() {
        let backend = MemoryBackend::new();
        backend.write("文件.txt", b"content").await.unwrap();
        let content = backend.read("文件.txt").await.unwrap();
        assert_eq!(content, b"content");
    }

    #[tokio::test]
    async fn test_unicode_content() {
        let backend = MemoryBackend::new();
        let unicode_content = "Hello 世界 🌍".as_bytes();
        backend.write("unicode.txt", unicode_content).await.unwrap();
        let content = backend.read("unicode.txt").await.unwrap();
        assert_eq!(content, unicode_content);
    }

    #[tokio::test]
    async fn test_default_impl() {
        let backend = MemoryBackend::default();
        backend.write("test.txt", b"hello").await.unwrap();
        let content = backend.read("test.txt").await.unwrap();
        assert_eq!(content, b"hello");
    }

    #[tokio::test]
    async fn test_rename() {
        let backend = MemoryBackend::new();
        backend.write("old.txt", b"content").await.unwrap();

        backend.rename("old.txt", "new.txt").await.unwrap();

        // Old path should not exist
        assert!(!backend.exists("old.txt").await.unwrap());
        // New path should exist with same content
        assert!(backend.exists("new.txt").await.unwrap());
        let content = backend.read("new.txt").await.unwrap();
        assert_eq!(content, b"content");
    }

    #[tokio::test]
    async fn test_rename_nonexistent() {
        let backend = MemoryBackend::new();
        let result = backend.rename("nonexistent.txt", "new.txt").await;
        assert!(matches!(result, Err(BackendError::NotFound(_))));
    }

    #[tokio::test]
    async fn test_rename_overwrites_existing() {
        let backend = MemoryBackend::new();
        backend.write("src.txt", b"source").await.unwrap();
        backend.write("dst.txt", b"destination").await.unwrap();

        backend.rename("src.txt", "dst.txt").await.unwrap();

        // Source should not exist
        assert!(!backend.exists("src.txt").await.unwrap());
        // Destination should have source content
        let content = backend.read("dst.txt").await.unwrap();
        assert_eq!(content, b"source");
    }
}
