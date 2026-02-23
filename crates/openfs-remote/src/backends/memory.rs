use std::collections::HashMap;
use std::sync::RwLock;

use async_trait::async_trait;
use chrono::{DateTime, Utc};

use openfs_core::{Backend, BackendError, Entry};

/// In-memory backend for testing.
pub struct MemoryBackend {
    files: RwLock<HashMap<String, (Vec<u8>, DateTime<Utc>, u64)>>,
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
            .map(|(content, _, _)| content.clone())
            .ok_or_else(|| BackendError::NotFound(path.to_string()))
    }

    async fn read_with_cas_token(
        &self,
        path: &str,
    ) -> Result<(Vec<u8>, Option<String>), BackendError> {
        let files = self.files.read().unwrap_or_else(|e| e.into_inner());
        let normalized = normalize_path(path);

        files
            .get(&normalized)
            .map(|(content, _, version)| (content.clone(), Some(version.to_string())))
            .ok_or_else(|| BackendError::NotFound(path.to_string()))
    }

    async fn write(&self, path: &str, content: &[u8]) -> Result<(), BackendError> {
        let mut files = self.files.write().unwrap_or_else(|e| e.into_inner());
        let normalized = normalize_path(path);
        let version = files
            .get(&normalized)
            .map(|(_, _, version)| version.saturating_add(1))
            .unwrap_or(1);
        files.insert(normalized, (content.to_vec(), Utc::now(), version));
        Ok(())
    }

    async fn compare_and_swap(
        &self,
        path: &str,
        expected: Option<&str>,
        content: &[u8],
    ) -> Result<Option<String>, BackendError> {
        let mut files = self.files.write().unwrap_or_else(|e| e.into_inner());
        let normalized = normalize_path(path);

        let current = files.get(&normalized).map(|(_, _, version)| *version);
        match (expected, current) {
            (Some(expected), Some(actual)) if expected != actual.to_string() => {
                return Err(BackendError::PreconditionFailed {
                    path: path.to_string(),
                    expected: expected.to_string(),
                    actual: actual.to_string(),
                });
            }
            (Some(expected), None) => {
                return Err(BackendError::PreconditionFailed {
                    path: path.to_string(),
                    expected: expected.to_string(),
                    actual: "absent".to_string(),
                });
            }
            _ => {}
        }

        let new_version = current.map(|v| v.saturating_add(1)).unwrap_or(1);
        files.insert(normalized, (content.to_vec(), Utc::now(), new_version));
        Ok(Some(new_version.to_string()))
    }

    async fn append(&self, path: &str, content: &[u8]) -> Result<(), BackendError> {
        let mut files = self.files.write().unwrap_or_else(|e| e.into_inner());
        let normalized = normalize_path(path);

        let entry = files
            .entry(normalized)
            .or_insert_with(|| (Vec::new(), Utc::now(), 0));
        entry.0.extend_from_slice(content);
        entry.1 = Utc::now();
        entry.2 = entry.2.saturating_add(1);
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

        for (file_path, (content, mtime, _)) in files.iter() {
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
                entries
                    .entry(first_component.to_string())
                    .or_insert_with(|| {
                        Entry::dir(
                            format!(
                                "{}{}",
                                if prefix.is_empty() { "" } else { &prefix },
                                first_component
                            ),
                            first_component.to_string(),
                            None,
                        )
                    });
            } else {
                // It's a file
                entries.insert(
                    first_component.to_string(),
                    Entry::file(
                        format!(
                            "{}{}",
                            if prefix.is_empty() { "" } else { &prefix },
                            first_component
                        ),
                        first_component.to_string(),
                        content.len() as u64,
                        Some(*mtime),
                    ),
                );
            }
        }

        let mut result: Vec<_> = entries.into_values().collect();
        result.sort_by(|a, b| match (a.is_dir, b.is_dir) {
            (true, false) => std::cmp::Ordering::Less,
            (false, true) => std::cmp::Ordering::Greater,
            _ => a.name.cmp(&b.name),
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
        if let Some((content, mtime, _)) = files.get(&normalized) {
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
            let name = normalized
                .rsplit('/')
                .next()
                .unwrap_or(&normalized)
                .to_string();
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
        backend
            .write("subdir/file3.txt", b"content3")
            .await
            .unwrap();

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
    async fn test_compare_and_swap_success_and_mismatch() {
        let backend = MemoryBackend::new();
        backend.write("test.txt", b"v1").await.unwrap();

        let (_content, token) = backend.read_with_cas_token("test.txt").await.unwrap();
        let token = token.expect("token should be present");

        let new_token = backend
            .compare_and_swap("test.txt", Some(&token), b"v2")
            .await
            .expect("cas should succeed")
            .expect("new token should be present");
        assert_ne!(token, new_token);

        let err = backend
            .compare_and_swap("test.txt", Some(&token), b"v3")
            .await
            .unwrap_err();
        assert!(matches!(err, BackendError::PreconditionFailed { .. }));
    }
}
